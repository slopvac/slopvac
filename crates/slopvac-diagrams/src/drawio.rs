//! draw.io extraction: plain XML, compressed pages, `.drawio.svg` and `.drawio.png`.
//!
//! A plain page keeps exact spans, so a label without entities or HTML is
//! fixable. A compressed page, an SVG `content` attribute and a PNG chunk hold
//! the XML in another encoding. A fragment from them reports the span of the
//! whole container and carries the converted text, so it is not fixable.

use std::io::Read;
use std::ops::Range;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use flate2::read::DeflateDecoder;
use roxmltree::{Document, Node, ParsingOptions};

use crate::{Extraction, Fragment, Role};

const MAX_INFLATED: u64 = 32 * 1024 * 1024;
const MAX_NODES: u32 = 1_000_000;

/// Where a parsed XML text sits in the host file.
#[derive(Clone)]
enum Origin {
    /// The XML is the host text at `base`; spans are exact.
    Direct(usize),
    /// The XML was decoded from `span`; every fragment reports that range.
    Whole(Range<usize>),
}

pub(crate) fn extract_xml(src: &str, base: usize) -> Extraction {
    let mut out = Vec::new();
    if !document(src, &Origin::Direct(base), &mut out) {
        return Extraction::default();
    }
    done(out)
}

pub(crate) fn extract_svg(src: &str, base: usize) -> Extraction {
    let Ok(doc) = parse(src) else {
        return Extraction::default();
    };
    let root = doc.root_element();
    let Some(attr) = root.attribute_node("content") else {
        return Extraction::default();
    };
    let range = attr.range_value();
    let whole = Origin::Whole(base + range.start..base + range.end);
    let mut out = Vec::new();
    if !document(attr.value(), &whole, &mut out) {
        return Extraction::default();
    }
    done(out)
}

pub(crate) fn extract_png(bytes: &[u8], base: usize) -> Extraction {
    let Some((range, text)) = png_mxfile(bytes) else {
        return Extraction::default();
    };
    let Some(xml) = percent_decode(&text) else {
        return Extraction::default();
    };
    let whole = Origin::Whole(base + range.start..base + range.end);
    let mut out = Vec::new();
    if !document(&xml, &whole, &mut out) {
        return Extraction::default();
    }
    done(out)
}

fn done(fragments: Vec<Fragment>) -> Extraction {
    Extraction {
        diagram: Some("drawio".to_string()),
        fragments,
    }
}

fn parse(src: &str) -> Result<Document<'_>, roxmltree::Error> {
    Document::parse_with_options(
        src,
        ParsingOptions {
            allow_dtd: true,
            nodes_limit: MAX_NODES,
            ..ParsingOptions::default()
        },
    )
}

/// Walk one XML document. Return false when it does not parse.
fn document(src: &str, origin: &Origin, out: &mut Vec<Fragment>) -> bool {
    let Ok(doc) = parse(src) else { return false };
    let root = doc.root_element();
    match root.tag_name().name() {
        "mxfile" => {
            for diagram in root.children().filter(|n| n.has_tag_name("diagram")) {
                let page = diagram.attribute("name").unwrap_or("page");
                if let Some(model) = diagram.children().find(|n| n.has_tag_name("mxGraphModel")) {
                    cells(model, page, origin, out);
                } else if let Some(payload) = diagram.text() {
                    let range = diagram.first_child().map(|c| c.range());
                    let whole = match (origin, range) {
                        (Origin::Direct(base), Some(r)) => {
                            Origin::Whole(base + r.start..base + r.end)
                        }
                        _ => origin.clone(),
                    };
                    let Some(xml) = inflate_page(payload) else {
                        return false;
                    };
                    if !document(&xml, &whole, out) {
                        return false;
                    }
                }
            }
            true
        }
        "mxGraphModel" => {
            cells(root, "page", origin, out);
            true
        }
        _ => false,
    }
}

fn cells(model: Node<'_, '_>, page: &str, origin: &Origin, out: &mut Vec<Fragment>) {
    for node in model.descendants().filter(Node::is_element) {
        let name = node.tag_name().name();
        let (label_attr, cell) = match name {
            "mxCell" if !wrapped(node) => ("value", node),
            "object" | "UserObject" => match node.children().find(|n| n.has_tag_name("mxCell")) {
                Some(cell) => ("label", cell),
                None => continue,
            },
            _ => continue,
        };
        let Some(attr) = node.attribute_node(label_attr) else {
            continue;
        };
        let id = node.attribute("id").unwrap_or("?");
        let style = cell.attribute("style").unwrap_or("");
        let role = role_of(cell, style);
        let value = attr.value();
        let converted = if style_flag(style, "html") {
            html_to_text(value)
        } else {
            value.trim().to_string()
        };
        if converted.is_empty() {
            continue;
        }
        let (span, text) = match origin {
            Origin::Direct(base) => {
                let r = attr.range_value();
                let span = base + r.start..base + r.end;
                let raw = &model.document().input_text()[r];
                (span, (raw != converted).then_some(converted))
            }
            Origin::Whole(span) => (span.clone(), Some(converted)),
        };
        out.push(Fragment {
            span,
            text,
            role,
            logical: format!("drawio/{page}/{}[{id}]", role.as_str()),
        });
    }
}

fn wrapped(cell: Node<'_, '_>) -> bool {
    cell.parent_element()
        .is_some_and(|p| matches!(p.tag_name().name(), "object" | "UserObject"))
}

fn style_flag(style: &str, key: &str) -> bool {
    style
        .split(';')
        .any(|part| part.trim() == format!("{key}=1"))
}

fn role_of(cell: Node<'_, '_>, style: &str) -> Role {
    let first = style.split(';').next().unwrap_or("").trim();
    if cell.attribute("edge") == Some("1") || style.split(';').any(|p| p.trim() == "edgeLabel") {
        Role::Edge
    } else if first == "text" || style_flag(style, "shape=text") || style.contains("shape=text") {
        Role::Text
    } else if first == "swimlane" || style_flag(style, "container") || first == "group" {
        Role::Group
    } else {
        Role::Node
    }
}

fn html_to_text(html: &str) -> String {
    let mut text = String::new();
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after
            .find('>')
            .filter(|_| after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/'));
        if let Some(close) = close {
            let tag = after[..close].trim_start_matches('/').to_ascii_lowercase();
            let name: String = tag
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            if name == "br"
                || (after.starts_with('/') && matches!(name.as_str(), "p" | "div" | "li"))
            {
                text.push('\n');
            }
            rest = &after[close + 1..];
        } else {
            text.push('<');
            rest = after;
        }
    }
    text.push_str(rest);
    let text = decode_entities(&text);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    lines.join("\n").trim().to_string()
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after.find(';').filter(|&n| n <= 8).and_then(|n| {
            let ch = match &after[..n] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => ' ',
                num => {
                    let code = num.strip_prefix('#')?;
                    let value = match code.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => code.parse().ok()?,
                    };
                    char::from_u32(value)?
                }
            };
            Some((ch, n))
        });
        if let Some((ch, n)) = decoded {
            out.push(ch);
            rest = &after[n + 1..];
        } else {
            out.push('&');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn inflate_page(payload: &str) -> Option<String> {
    let compact: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = STANDARD_NO_PAD.decode(compact.trim_end_matches('=')).ok()?;
    let mut inflated = Vec::new();
    DeflateDecoder::new(bytes.as_slice())
        .take(MAX_INFLATED + 1)
        .read_to_end(&mut inflated)
        .ok()?;
    if inflated.len() as u64 > MAX_INFLATED {
        return None;
    }
    percent_decode(&inflated)
}

fn percent_decode(bytes: &[u8]) -> Option<String> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Find the `mxfile` text chunk of a PNG. Return its data range and bytes.
fn png_mxfile(bytes: &[u8]) -> Option<(Range<usize>, Vec<u8>)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return None;
    }
    let mut at = SIGNATURE.len();
    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().ok()?) as usize;
        let kind = &bytes[at + 4..at + 8];
        let data = at + 8..at + 8 + len;
        if data.end + 4 > bytes.len() {
            return None;
        }
        if kind == b"tEXt" {
            let chunk = &bytes[data.clone()];
            if let Some(nul) = chunk.iter().position(|&b| b == 0)
                && &chunk[..nul] == b"mxfile"
            {
                return Some((data, chunk[nul + 1..].to_vec()));
            }
        }
        if kind == b"IEND" {
            return None;
        }
        at = data.end + 4;
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::{Format, Role, extract, extract_bytes};

    const COMPRESSED: &str = "jU9bCgMhDDxN/tXcoPbx1UMIBlqIRqJb7O3r1oWWwkI/ApOZTJgB9KlfNJTbVSIx4AnQq0ibKHVPzODMPQIewTkzBtx5R7Vv1ZSglNs/BjcNj8ALTcZLKkq1Uhx8HpnmRW1P3i5UlhxpfWABD9sD0kZ9N4T9DjHAp+BYfvq/AA==";
    const PNG_TEXT: &str = "%3Cmxfile%3E%3Cdiagram%20name%3D%22P%22%3E%3CmxGraphModel%3E%3Croot%3E%3CmxCell%20id%3D%222%22%20value%3D%22In%20a%20PNG%22%20vertex%3D%221%22%20parent%3D%221%22%2F%3E%3C%2Froot%3E%3C%2FmxGraphModel%3E%3C%2Fdiagram%3E%3C%2Fmxfile%3E";

    const PLAIN: &str = r#"<mxfile><diagram name="Main"><mxGraphModel><root>
<mxCell id="0"/><mxCell id="1" parent="0"/>
<mxCell id="a" value="API gateway" style="rounded=1;" vertex="1" parent="1"/>
<mxCell id="b" value="Reads &amp; writes" style="edgeStyle=orthogonalEdgeStyle;" edge="1" parent="1"/>
<mxCell id="c" value="&lt;b&gt;Legend&lt;/b&gt;&lt;br&gt;blue is new" style="text;html=1;" vertex="1" parent="1"/>
<mxCell id="d" value="Backend" style="swimlane;" vertex="1" parent="1"/>
<object label="Wrapped label" id="e"><mxCell style="rounded=0;" vertex="1" parent="1"/></object>
<mxCell id="f" value="" vertex="1" parent="1"/>
</root></mxGraphModel></diagram></mxfile>"#;

    fn found(src: &str) -> Vec<(Role, String, bool)> {
        extract(Format::Drawio, src, 0)
            .fragments
            .into_iter()
            .map(|f| {
                (
                    f.role,
                    f.text
                        .clone()
                        .unwrap_or_else(|| src[f.span.clone()].to_string()),
                    f.fixable(),
                )
            })
            .collect()
    }

    #[test]
    fn plain_xml_roles_text_and_fixability() {
        assert_eq!(
            found(PLAIN),
            [
                (Role::Node, "API gateway".to_string(), true),
                (Role::Edge, "Reads & writes".to_string(), false),
                (Role::Text, "Legend\nblue is new".to_string(), false),
                (Role::Group, "Backend".to_string(), true),
                (Role::Node, "Wrapped label".to_string(), true),
            ]
        );
    }

    #[test]
    fn plain_span_is_the_attribute_value_bytes() {
        let ex = extract(Format::Drawio, PLAIN, 7);
        let start = PLAIN.find("API gateway").unwrap() + 7;
        assert_eq!(ex.fragments[0].span, start..start + "API gateway".len());
        assert_eq!(ex.fragments[0].logical, "drawio/Main/node[a]");
    }

    #[test]
    fn compressed_page_reports_the_payload_span() {
        let src = format!("<mxfile><diagram name=\"Z\">{COMPRESSED}</diagram></mxfile>");
        let ex = extract(Format::Drawio, &src, 0);
        assert_eq!(ex.fragments.len(), 1);
        let f = &ex.fragments[0];
        assert_eq!(f.text.as_deref(), Some("Compressed node"));
        assert_eq!(&src[f.span.clone()], COMPRESSED);
        assert!(!f.fixable());
    }

    #[test]
    fn svg_content_attribute_is_decoded() {
        let inner = format!("<mxfile><diagram name=\"S\">{COMPRESSED}</diagram></mxfile>");
        let escaped = inner
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        let svg =
            format!("<svg xmlns=\"http://www.w3.org/2000/svg\" content=\"{escaped}\"><g/></svg>");
        let ex = extract(Format::DrawioSvg, &svg, 0);
        assert_eq!(ex.fragments[0].text.as_deref(), Some("Compressed node"));
        assert_eq!(&svg[ex.fragments[0].span.clone()], escaped);
    }

    #[test]
    fn png_text_chunk_is_decoded() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut data = b"mxfile\0".to_vec();
        data.extend_from_slice(PNG_TEXT.as_bytes());
        png.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        png.extend_from_slice(b"tEXt");
        let start = png.len();
        png.extend_from_slice(&data);
        png.extend_from_slice(&[0; 4]);
        let ex = extract_bytes(Format::DrawioPng, &png, 0);
        assert_eq!(ex.fragments[0].text.as_deref(), Some("In a PNG"));
        assert_eq!(ex.fragments[0].span, start..start + data.len());
    }

    #[test]
    fn broken_input_yields_nothing() {
        for bad in [
            "<mxfile",
            "<svg/>",
            "<mxfile><diagram>!!!notbase64</diagram></mxfile>",
        ] {
            assert_eq!(
                extract(Format::Drawio, bad, 0).fragments,
                Vec::new(),
                "{bad}"
            );
        }
        assert_eq!(
            extract_bytes(Format::DrawioPng, b"not a png", 0).diagram,
            None
        );
    }

    #[test]
    fn external_entities_are_refused() {
        let xxe = "<!DOCTYPE x [<!ENTITY e SYSTEM \"file:///etc/passwd\">]><mxGraphModel><root><mxCell id=\"2\" value=\"&e;\" vertex=\"1\"/></root></mxGraphModel>";
        assert_eq!(extract(Format::Drawio, xxe, 0).diagram, None);
    }
}
