//! Find diagram blocks in Markdown and MDX.
//!
//! The blocks come from the code blocks of `slopvac-parse`, so fences in lists,
//! block quotes and MDX follow the same rules as the prose engine.

use std::borrow::Cow;
use std::ops::Range;

use slopvac_model::Span;
use slopvac_parse::{BlockKind, Document, ParseError, parse_markdown, parse_mdx};

use crate::{Extraction, Format};

/// A fenced code block that holds a diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramBlock {
    /// The format named by the info string.
    pub format: Format,
    /// Byte range of the block body, between the fence lines.
    pub body: Range<usize>,
}

/// Find the diagram fences of a Markdown document.
///
/// # Errors
/// Returns the parser's error when the document does not parse.
pub fn find_markdown_blocks(markdown: &str) -> Result<Vec<DiagramBlock>, ParseError> {
    let source = lone_cr_as_lf(markdown);
    Ok(diagram_blocks(&parse_markdown(&source, whole(&source))?))
}

/// Find the diagram fences of an MDX document.
///
/// # Errors
/// Returns the parser's error when the document is not valid MDX.
pub fn find_mdx_blocks(mdx: &str) -> Result<Vec<DiagramBlock>, ParseError> {
    let source = lone_cr_as_lf(mdx);
    Ok(diagram_blocks(&parse_mdx(&source, whole(&source))?))
}

/// `slopvac-parse` does not treat a lone CR as a line ending in fences, but the
/// positions contract does. Swapping each CR that does not precede LF for LF
/// keeps the byte length, so every offset still addresses the original text.
fn lone_cr_as_lf(source: &str) -> Cow<'_, str> {
    let bytes = source.as_bytes();
    let lone = |i: usize| bytes[i] == b'\r' && bytes.get(i + 1) != Some(&b'\n');
    if !(0..bytes.len()).any(lone) {
        return Cow::Borrowed(source);
    }
    // CR and LF are ASCII, so the swap keeps the text valid UTF-8.
    Cow::Owned(
        source
            .char_indices()
            .map(|(i, c)| if c == '\r' && lone(i) { '\n' } else { c })
            .collect(),
    )
}

/// Extract a diagram fence body, with spans in the host `source`.
///
/// `body` is the range from [`DiagramBlock`]. It starts after the container prefix of its
/// first line, but later lines still carry theirs (`> ` of a block quote, the indent of a
/// list item). The prefix is stripped from each line before extraction, so a label that
/// leads a line is not hidden behind a quote marker. Spans map back through the stripped
/// bytes. A fragment that runs over a stripped prefix has no byte-for-byte replacement, so
/// it carries its converted text and is not fixable.
#[must_use]
pub fn extract_fenced(format: Format, source: &str, body: Range<usize>) -> Extraction {
    let Some(text) = source.get(body.clone()) else {
        return Extraction::default();
    };
    let strip = Prefix::of_line_before(source, body.start);
    if strip.is_empty() {
        return crate::extract(format, text, body.start);
    }
    let mut stripped = String::with_capacity(text.len());
    // (offset in `stripped`, offset in `source`) for the start of each kept line piece.
    let mut pieces: Vec<(usize, usize)> = Vec::new();
    let mut line_start = 0;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let skip = if i == 0 { 0 } else { strip.len_in(line) };
        pieces.push((stripped.len(), body.start + line_start + skip));
        stripped.push_str(&line[skip..]);
        line_start += line.len();
    }
    let map = |at: usize| {
        let i = pieces.partition_point(|&(s, _)| s <= at).saturating_sub(1);
        pieces.get(i).map_or(at, |&(s, o)| o + (at - s))
    };
    let mut out = crate::extract(format, &stripped, 0);
    for f in &mut out.fragments {
        let (a, b) = (f.span.start, f.span.end);
        let start = map(a);
        let end = if b > a { map(b - 1) + 1 } else { start };
        if f.text.is_none() && end - start != b - a {
            f.text = Some(stripped[a..b].to_owned());
        }
        f.span = start..end;
    }
    out
}

/// The container prefix of a body's first line, as a rule for the lines after it.
struct Prefix {
    quotes: usize,
    indent: usize,
}

impl Prefix {
    fn of_line_before(source: &str, at: usize) -> Self {
        let line_start = source[..at].rfind(['\n', '\r']).map_or(0, |i| i + 1);
        let prefix = &source[line_start..at];
        let none = Self {
            quotes: 0,
            indent: 0,
        };
        if !prefix.bytes().all(|b| matches!(b, b' ' | b'\t' | b'>')) {
            return none;
        }
        let quotes = prefix.bytes().filter(|&b| b == b'>').count();
        let tail = prefix.rfind('>').map_or(prefix, |i| &prefix[i + 1..]);
        let indent = if quotes > 0 {
            tail.len().saturating_sub(1)
        } else {
            tail.len()
        };
        Self { quotes, indent }
    }

    const fn is_empty(&self) -> bool {
        self.quotes == 0 && self.indent == 0
    }

    /// Bytes of `line` that belong to the container.
    fn len_in(&self, line: &str) -> usize {
        let b = line.as_bytes();
        let mut at = 0;
        for _ in 0..self.quotes {
            let mut j = at;
            while j < b.len() && j < at + 3 && b[j] == b' ' {
                j += 1;
            }
            if b.get(j) != Some(&b'>') {
                return at;
            }
            at = j + 1;
            if b.get(at) == Some(&b' ') {
                at += 1;
            }
        }
        let spaces = b[at..]
            .iter()
            .take(self.indent)
            .take_while(|&&c| c == b' ')
            .count();
        at + spaces
    }
}

fn whole(source: &str) -> Span {
    Span::new(0, source.len()).expect("zero never exceeds the length")
}

fn diagram_blocks(document: &Document) -> Vec<DiagramBlock> {
    document
        .blocks
        .iter()
        .filter_map(|block| match &block.kind {
            BlockKind::CodeBlock {
                lang: Some(lang),
                body,
                fenced: true,
                ..
            } => Format::from_lang(lang).map(|format| DiagramBlock {
                format,
                body: body.start()..body.end(),
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bodies(md: &str) -> Vec<(Format, &str)> {
        find_markdown_blocks(md)
            .unwrap()
            .into_iter()
            .map(|b| (b.format, &md[b.body]))
            .collect()
    }

    #[test]
    fn finds_body_between_fences() {
        let md = "# T\n\n```mermaid\ngraph TD\n  A-->B\n```\n\ntext\n";
        assert_eq!(bodies(md), [(Format::Mermaid, "graph TD\n  A-->B\n")]);
    }

    #[test]
    fn ignores_other_languages_and_nested_shorter_fences() {
        let md = "````text\n```mermaid\nx\n```\n````\n~~~mermaid\nA\n~~~\n";
        assert_eq!(bodies(md), [(Format::Mermaid, "A\n")]);
    }

    #[test]
    fn tilde_fence_with_plantuml_info_words() {
        let md = "~~~plantuml title=x\n@startuml\nA -> B\n@enduml\n~~~\n";
        assert_eq!(
            bodies(md),
            [(Format::PlantUml, "@startuml\nA -> B\n@enduml\n")]
        );
    }

    #[test]
    fn indented_apparent_fence_is_not_a_diagram() {
        assert_eq!(bodies("    ```mermaid\n    A-->B\n    ```\n"), []);
    }

    #[test]
    fn unterminated_fence_runs_to_the_end_of_the_document() {
        let md = "```mermaid\nA-->B\n";
        let blocks = find_markdown_blocks(md).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].format, Format::Mermaid);
        assert_eq!(blocks[0].body, 11..md.len());
    }

    #[test]
    fn mixed_and_lone_cr_endings_keep_original_offsets() {
        let md = "x\r\n```mermaid\rA-->B\r```\r\n";
        assert_eq!(bodies(md), [(Format::Mermaid, "A-->B\r")]);
    }

    #[test]
    fn mdx_lone_cr_endings_keep_original_offsets() {
        let mdx = "x\r\n```mermaid\rA-->B\r```\r\n";
        let blocks = find_mdx_blocks(mdx).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(&mdx[blocks[0].body.clone()], "A-->B\r");
    }

    #[test]
    fn handles_crlf_with_original_offsets() {
        let md = "x\r\n\r\n```mermaid\r\nA-->B\r\n```\r\n";
        assert_eq!(bodies(md), [(Format::Mermaid, "A-->B\r\n")]);
    }

    #[test]
    fn fence_in_a_list_item_keeps_container_prefixes_between_lines() {
        let md = "- item\n\n  ```mermaid\n  A-->B\n  B-->C\n  ```\n";
        // The body runs from the first to the last body line; the two-space
        // prefix of the second line sits inside the range.
        assert_eq!(bodies(md), [(Format::Mermaid, "A-->B\n  B-->C\n")]);
    }

    #[test]
    fn fence_in_a_block_quote_keeps_quote_markers_between_lines() {
        let md = "> ```mermaid\n> A-->B\n> B-->C\n> ```\n";
        assert_eq!(bodies(md), [(Format::Mermaid, "A-->B\n> B-->C\n")]);
    }

    #[test]
    fn fence_in_a_list_inside_a_block_quote() {
        let md = "> 1. item\n>\n>    ~~~plantuml\n>    A -> B\n>    ~~~\n";
        assert_eq!(bodies(md), [(Format::PlantUml, "A -> B\n")]);
    }

    #[test]
    fn mdx_fence_after_jsx_and_in_a_list() {
        let mdx =
            "<Note>hi</Note>\n\n```mermaid\nA-->B\n```\n\n- x\n\n  ~~~mermaid\n  C-->D\n  ~~~\n";
        let got: Vec<_> = find_mdx_blocks(mdx)
            .unwrap()
            .into_iter()
            .map(|b| &mdx[b.body])
            .collect();
        assert_eq!(got, ["A-->B\n", "C-->D\n"]);
    }

    #[test]
    fn mdx_that_does_not_parse_is_an_error() {
        assert!(find_mdx_blocks("<Open>\n").is_err());
    }

    fn labels(md: &str) -> Vec<(&str, Range<usize>)> {
        let block = &find_markdown_blocks(md).unwrap()[0];
        extract_fenced(block.format, md, block.body.clone())
            .fragments
            .into_iter()
            .map(|f| (&md[f.span.clone()], f.span))
            .collect()
    }

    #[test]
    fn block_quote_labels_are_found_at_their_source_offsets() {
        let md = "> ```mermaid\n> sequenceDiagram\n> participant A as Alice\n> ```\n";
        // "> ```mermaid\n" is 13 bytes and "> sequenceDiagram\n" is 18, so line 3 starts at 31.
        // The prefix is 2 bytes and `participant A as ` is 17, so "Alice" is 50..55.
        assert_eq!(labels(md), [("Alice", 50..55)]);
    }

    #[test]
    fn keyword_led_plantuml_line_in_a_quote_is_not_dropped() {
        let md = "> ```plantuml\n> @startuml\n> title Hello\n> @enduml\n> ```\n";
        // "> ```plantuml\n" is 14 bytes and "> @startuml\n" is 12, so line 3 starts at 26.
        // The prefix is 2 bytes and `title ` is 6, so "Hello" is 34..39.
        assert_eq!(labels(md), [("Hello", 34..39)]);
    }

    #[test]
    fn nested_list_in_quote_labels_keep_source_offsets() {
        let md = "> - item\n>\n>   ```mermaid\n>   sequenceDiagram\n>   participant A as Alice\n>   ```\n";
        // "> - item\n" 9, ">\n" 2, ">   ```mermaid\n" 15 and ">   sequenceDiagram\n" 20 put
        // line 5 at byte 46. Its prefix is 4 bytes and `participant A as ` is 17, so
        // "Alice" is 67..72.
        assert_eq!(labels(md), [("Alice", 67..72)]);
    }
}
