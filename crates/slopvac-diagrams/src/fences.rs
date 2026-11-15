//! Find diagram blocks in Markdown and MDX.
//!
//! The blocks come from the code blocks of `slopvac-parse`, so fences in lists,
//! block quotes and MDX follow the same rules as the prose engine.

use std::ops::Range;

use slopvac_model::Span;
use slopvac_parse::{BlockKind, Document, ParseError, parse_markdown, parse_mdx};

use crate::Format;

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
    Ok(diagram_blocks(&parse_markdown(markdown, whole(markdown))?))
}

/// Find the diagram fences of an MDX document.
///
/// # Errors
/// Returns the parser's error when the document is not valid MDX.
pub fn find_mdx_blocks(mdx: &str) -> Result<Vec<DiagramBlock>, ParseError> {
    Ok(diagram_blocks(&parse_mdx(mdx, whole(mdx))?))
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
    fn indented_code_is_not_a_diagram() {
        assert_eq!(bodies("    mermaid\n    A-->B\n"), []);
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
}
