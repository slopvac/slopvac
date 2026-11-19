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

use crate::sink::{CAP_INFLATED_BYTES, CAP_XML_NODES, Sink, clip};
use crate::{BudgetHit, Extraction, ExtractionStatus, Fragment, Role};

const MAX_INFLATED: u64 = 32 * 1024 * 1024;
const MAX_NODES: u32 = 1_000_000;

/// Limits shared by every page and every nested document of one file, so a file
/// of many small pages cannot add up to more than one large page.
struct Budget {
    /// Inflated bytes left.
    inflated: u64,
    /// XML nodes left.
    nodes: u32,
}

impl Budget {
    fn new() -> Self {
        Budget {
            inflated: MAX_INFLATED,
            nodes: MAX_NODES,
        }
    }
}

/// Where a parsed XML text sits in the host file.
#[derive(Clone)]
enum Origin {
    /// The XML is the host text at `base`; spans are exact.
    Direct(usize),
    /// The XML was decoded from `span`; every fragment reports that range.
    Whole(Range<usize>),
}

/// Why a read stopped. The fragments read before it stay in the output.
type Failure = ExtractionStatus;

pub(crate) fn extract_xml(src: &str, base: usize) -> Extraction {
    extract_xml_with(src, base, &mut Budget::new())
}

fn extract_xml_with(src: &str, base: usize, budget: &mut Budget) -> Extraction {
    let mut out = Sink::default();
    let result = document(src, &Origin::Direct(base), budget, &mut out);
    done(out, result)
}

pub(crate) fn extract_svg(src: &str, base: usize) -> Extraction {
    extract_svg_with(src, base, &mut Budget::new())
}

fn extract_svg_with(src: &str, base: usize, budget: &mut Budget) -> Extraction {
    let doc = match parse(src, budget) {
        Ok(doc) => doc,
        Err(status) => return Extraction::failed(status),
    };
    let root = doc.root_element();
    let Some(attr) = root.attribute_node("content") else {
        return Extraction::failed(ExtractionStatus::Unsupported("no embedded diagram"));
    };
    let range = attr.range_value();
    let whole = Origin::Whole(base + range.start..base + range.end);
    let mut out = Sink::default();
    let result = document(attr.value(), &whole, budget, &mut out);
    done(out, result)
}

pub(crate) fn extract_png(bytes: &[u8], base: usize) -> Extraction {
    extract_png_with(bytes, base, &mut Budget::new())
}

fn extract_png_with(bytes: &[u8], base: usize, budget: &mut Budget) -> Extraction {
    let Some((range, text)) = png_mxfile(bytes) else {
        return Extraction::failed(ExtractionStatus::Unsupported("no embedded diagram"));
    };
    let Some(xml) = percent_decode(&text) else {
        return Extraction::failed(ExtractionStatus::Malformed("invalid percent encoding"));
    };
    let whole = Origin::Whole(base + range.start..base + range.end);
    let mut out = Sink::default();
    let result = document(&xml, &whole, budget, &mut out);
    done(out, result)
}

/// Build the extraction. A failure keeps the fragments read so far.
fn done(out: Sink, result: Result<(), Failure>) -> Extraction {
    let (fragments, hit) = out.finish();
    Extraction {
        diagram: Some("drawio".to_string()),
        fragments,
        status: result.err().unwrap_or_else(|| crate::sink::status(hit)),
    }
}

/// Parse XML against the shared node budget.
fn parse<'a>(src: &'a str, budget: &mut Budget) -> Result<Document<'a>, Failure> {
    let doc = Document::parse_with_options(
        src,
        ParsingOptions {
            allow_dtd: true,
            nodes_limit: budget.nodes,
            ..ParsingOptions::default()
        },
    )
    .map_err(|e| match e {
        roxmltree::Error::NodesLimitReached => {
            ExtractionStatus::BudgetExhausted(BudgetHit::new(CAP_XML_NODES, u64::from(MAX_NODES)))
        }
        _ => ExtractionStatus::Malformed("invalid XML"),
    })?;
    let used = u32::try_from(doc.descendants().count()).unwrap_or(u32::MAX);
    budget.nodes = budget.nodes.saturating_sub(used);
    Ok(doc)
}

/// Walk one XML document. A page that fails stops the walk, and earlier pages stay in `out`.
fn document(
    src: &str,
    origin: &Origin,
    budget: &mut Budget,
    out: &mut Sink,
) -> Result<(), Failure> {
    let doc = parse(src, budget)?;
    let root = doc.root_element();
    match root.tag_name().name() {
        "mxfile" => {
            let has_content = root
                .children()
                .any(|n| n.is_element() || n.text().is_some_and(|t| !t.trim().is_empty()));
            if has_content && !root.children().any(|n| n.has_tag_name("diagram")) {
                return Err(ExtractionStatus::Malformed("no diagram pages"));
            }
            for diagram in root.children().filter(|n| n.has_tag_name("diagram")) {
                if out.is_full() {
                    break;
                }
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
                    let xml = inflate_page(payload, budget)?;
                    inflated_model(&xml, page, &whole, budget, out)?;
                } else if diagram.children().any(|n| n.is_element()) {
                    return Err(ExtractionStatus::Malformed("unrecognised page content"));
                }
            }
            Ok(())
        }
        "mxGraphModel" => {
            cells(root, "page", origin, out);
            Ok(())
        }
        _ => Err(ExtractionStatus::Unsupported("not a draw.io document")),
    }
}

/// Walk an inflated page. Its root must be the graph model: a page does not nest another file.
fn inflated_model(
    xml: &str,
    page: &str,
    origin: &Origin,
    budget: &mut Budget,
    out: &mut Sink,
) -> Result<(), Failure> {
    let doc = parse(xml, budget)?;
    let root = doc.root_element();
    if !root.has_tag_name("mxGraphModel") {
        return Err(ExtractionStatus::Malformed("page is not a graph model"));
    }
    cells(root, page, origin, out);
    Ok(())
}

fn cells(model: Node<'_, '_>, page: &str, origin: &Origin, out: &mut Sink) {
    for node in model.descendants().filter(Node::is_element) {
        if out.is_full() {
            return;
        }
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
            logical: format!("drawio/{}/{}[{}]", clip(page), role.as_str(), clip(id)),
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
    html_to_text_counted(html, &mut 0)
}

/// Convert HTML to text. `work` adds the bytes that each search examined.
fn html_to_text_counted(html: &str, work: &mut usize) -> String {
    let mut text = String::new();
    let mut rest = html;
    // No tag can close after the last `>`, so one reverse scan bounds every later search.
    let last_close = html.rfind('>');
    while let Some(open) = rest.find('<') {
        *work += open + 1;
        text.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let consumed = html.len() - rest.len() + open + 1;
        let starts_tag = after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/');
        let close = if starts_tag && last_close.is_some_and(|last| last >= consumed) {
            let found = after.find('>');
            *work += found.map_or(after.len(), |c| c + 1);
            found
        } else {
            None
        };
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
    *work += rest.len();
    text.push_str(rest);
    let text = decode_entities(&text, work);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    lines.join("\n").trim().to_string()
}

fn decode_entities(s: &str, work: &mut usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        *work += amp + 1;
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let window = &after.as_bytes()[..after.len().min(9)];
        *work += window.len();
        let decoded = window.iter().position(|&b| b == b';').and_then(|n| {
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

fn inflate_page(payload: &str, budget: &mut Budget) -> Result<String, Failure> {
    let compact: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = STANDARD_NO_PAD
        .decode(compact.trim_end_matches('='))
        .map_err(|_| ExtractionStatus::Malformed("invalid base64 page"))?;
    let mut inflated = Vec::new();
    DeflateDecoder::new(bytes.as_slice())
        .take(budget.inflated + 1)
        .read_to_end(&mut inflated)
        .map_err(|_| ExtractionStatus::Malformed("invalid compressed page"))?;
    let size = inflated.len() as u64;
    if size > budget.inflated {
        return Err(ExtractionStatus::BudgetExhausted(BudgetHit::new(
            CAP_INFLATED_BYTES,
            MAX_INFLATED,
        )));
    }
    budget.inflated -= size;
    percent_decode(&inflated).ok_or(ExtractionStatus::Malformed("invalid percent encoding"))
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
        let data = at + 8..at.checked_add(8)?.checked_add(len)?;
        if data.end.checked_add(4)? > bytes.len() {
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
    use crate::{ExtractionStatus, Format, Role, extract, extract_bytes};

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
    fn broken_input_reports_why_it_failed() {
        for (bad, want) in [
            ("<mxfile", ExtractionStatus::Malformed("invalid XML")),
            (
                "<svg/>",
                ExtractionStatus::Unsupported("not a draw.io document"),
            ),
            (
                "<mxfile><diagram>!!!notbase64</diagram></mxfile>",
                ExtractionStatus::Malformed("invalid base64 page"),
            ),
        ] {
            let ex = extract(Format::Drawio, bad, 0);
            assert_eq!(ex.status, want, "{bad}");
            assert!(ex.fragments.is_empty(), "{bad}");
        }
        assert_eq!(
            extract_bytes(Format::DrawioPng, b"not a png", 0).status,
            ExtractionStatus::Unsupported("no embedded diagram")
        );
    }

    #[test]
    fn a_file_with_no_labels_is_ok() {
        let ex = extract(Format::Drawio, "<mxfile><diagram name=\"A\"/></mxfile>", 0);
        assert_eq!(ex.status, ExtractionStatus::Ok);
        assert_eq!(ex.fragments, Vec::new());
    }

    #[test]
    fn an_svg_without_an_embedded_diagram_is_unsupported() {
        let ex = extract(
            Format::DrawioSvg,
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            0,
        );
        assert_eq!(
            ex.status,
            ExtractionStatus::Unsupported("no embedded diagram")
        );
    }

    #[test]
    fn a_bad_later_page_keeps_the_earlier_pages_fragments() {
        let src = "<mxfile><diagram name=\"One\"><mxGraphModel><root><mxCell id=\"a\" value=\"Kept\" vertex=\"1\"/></root></mxGraphModel></diagram>\
             <diagram name=\"Two\">!!!notbase64</diagram></mxfile>";
        let ex = extract(Format::Drawio, src, 0);
        assert_eq!(
            ex.status,
            ExtractionStatus::Malformed("invalid base64 page")
        );
        let logical: Vec<_> = ex.fragments.iter().map(|f| f.logical.as_str()).collect();
        assert_eq!(logical, ["drawio/One/node[a]"]);
    }

    #[test]
    fn external_entities_are_refused() {
        let xxe = "<!DOCTYPE x [<!ENTITY e SYSTEM \"file:///etc/passwd\">]><mxGraphModel><root><mxCell id=\"2\" value=\"&e;\" vertex=\"1\"/></root></mxGraphModel>";
        assert_eq!(
            extract(Format::Drawio, xxe, 0).status,
            ExtractionStatus::Malformed("invalid XML")
        );
    }
}

#[cfg(test)]
mod limit_tests {
    use std::fmt::Write as _;
    use std::io::Write as _;

    use base64::engine::general_purpose::STANDARD;
    use flate2::Compression;
    use flate2::write::DeflateEncoder;

    use super::*;

    /// Deflate and base64 `xml` the way draw.io stores a compressed page.
    fn compressed_page(xml: &str) -> String {
        let encoded: String = xml
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() {
                    char::from(b).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(encoded.as_bytes()).unwrap();
        STANDARD.encode(enc.finish().unwrap())
    }

    fn model(cells: usize) -> String {
        let mut body = String::new();
        for i in 0..cells {
            write!(body, "<mxCell id=\"c{i}\" value=\"L{i}\" vertex=\"1\"/>").unwrap();
        }
        format!("<mxGraphModel><root>{body}</root></mxGraphModel>")
    }

    fn file(pages: &[String]) -> String {
        let mut body = String::new();
        for (i, p) in pages.iter().enumerate() {
            write!(body, "<diagram name=\"P{i}\">{p}</diagram>").unwrap();
        }
        format!("<mxfile>{body}</mxfile>")
    }

    #[test]
    fn inflation_budget_is_shared_across_pages() {
        let page = compressed_page(&model(50));
        let src = file(&[page.clone(), page]);
        let one_page_inflated = model(50).len() as u64 * 3;
        let mut roomy = Budget {
            inflated: one_page_inflated * 2 + 1024,
            nodes: MAX_NODES,
        };
        assert_eq!(extract_xml_with(&src, 0, &mut roomy).fragments.len(), 100);
        let mut tight = Budget {
            inflated: one_page_inflated + 64,
            nodes: MAX_NODES,
        };
        let ex = extract_xml_with(&src, 0, &mut tight);
        assert_eq!(
            ex.status,
            ExtractionStatus::BudgetExhausted(BudgetHit::new("inflated-bytes", MAX_INFLATED))
        );
        // The first page of 50 cells fits the budget and stays.
        assert_eq!(ex.fragments.len(), 50);
    }

    #[test]
    fn a_page_that_inflates_past_the_budget_is_refused() {
        let blank = " ".repeat(200_000);
        let xml = format!("{}{blank}", model(1));
        let src = file(&[compressed_page(&xml)]);
        let mut budget = Budget {
            inflated: 100_000,
            nodes: MAX_NODES,
        };
        assert_eq!(
            extract_xml_with(&src, 0, &mut budget).status,
            ExtractionStatus::BudgetExhausted(BudgetHit::new("inflated-bytes", MAX_INFLATED))
        );
    }

    #[test]
    fn node_budget_is_shared_across_pages() {
        let page = compressed_page(&model(30));
        let src = file(&[page.clone(), page]);
        let mut roomy = Budget {
            inflated: MAX_INFLATED,
            nodes: 500,
        };
        assert_eq!(extract_xml_with(&src, 0, &mut roomy).fragments.len(), 60);
        let mut tight = Budget {
            inflated: MAX_INFLATED,
            nodes: 50,
        };
        let ex = extract_xml_with(&src, 0, &mut tight);
        assert_eq!(
            ex.status,
            ExtractionStatus::BudgetExhausted(BudgetHit::new("xml-nodes", 1_000_000))
        );
        // The outer file takes 6 nodes and the first page 34, within 50. The second page does not fit.
        assert_eq!(ex.fragments.len(), 30);
    }

    #[test]
    fn a_compressed_page_cannot_nest_another_file() {
        let inner = file(&[compressed_page(&model(1))]);
        let src = file(&[compressed_page(&inner)]);
        assert_eq!(
            extract_xml(&src, 0).status,
            ExtractionStatus::Malformed("page is not a graph model")
        );
    }

    #[test]
    fn unmatched_markup_openers_examine_a_linear_number_of_bytes() {
        for unit in ["<1 ", "<a ", "&x", "&amp", "</", "<a>&"] {
            let html = unit.repeat(20_000);
            let mut work = 0;
            let _ = html_to_text_counted(&html, &mut work);
            assert!(
                work <= 16 * html.len(),
                "{unit:?}: examined {work} bytes of {}",
                html.len()
            );
        }
    }

    #[test]
    fn unmatched_openers_keep_their_text_and_entities_decode() {
        let lt = "<1 ".repeat(1_000);
        assert_eq!(html_to_text(&lt), lt.trim());
        let tags = "<a ".repeat(1_000);
        assert_eq!(html_to_text(&tags), tags.trim());
        let amp = "&x".repeat(1_000);
        assert_eq!(html_to_text(&amp), amp);
        assert_eq!(
            html_to_text("a &amp; b &#65; &bogus; c"),
            "a & b A &bogus; c"
        );
    }

    fn png(chunks: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in chunks {
            out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&[0; 4]);
        }
        out
    }

    #[test]
    fn malformed_png_chunks_fail_without_panicking() {
        let mut max_len = png(&[]);
        max_len.extend_from_slice(&u32::MAX.to_be_bytes());
        max_len.extend_from_slice(b"tEXt");
        max_len.extend_from_slice(b"mxfile\0abc");
        let mut cut_header = png(&[]);
        cut_header.extend_from_slice(&[0, 0, 0, 5, b't', b'E']);
        let mut cut_crc = png(&[(b"tEXt", b"mxfile\0abc")]);
        cut_crc.truncate(cut_crc.len() - 2);
        let no_nul = png(&[(b"tEXt", b"mxfile")]);
        let other_key = png(&[(b"tEXt", b"Comment\0hello")]);
        let bad_percent = png(&[(b"tEXt", b"mxfile\0%zz")]);
        for (name, bytes) in [
            ("max length", max_len),
            ("cut header", cut_header),
            ("cut crc", cut_crc),
            ("no nul", no_nul),
            ("other key", other_key),
            ("bad percent", bad_percent),
        ] {
            assert!(extract_png(&bytes, 0).status.is_failure(), "{name}");
            assert!(extract_png(&bytes, 0).fragments.is_empty(), "{name}");
        }
    }
}
