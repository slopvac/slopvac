//! Find diagram blocks in Markdown.
//!
//! STUB(slopvac-8l4.10.5): a minimal fenced-code scanner that stands in for the
//! code-block API of `slopvac-parse`. It knows backtick and tilde fences with a
//! language word, and nothing about block quotes or MDX.

use std::ops::Range;

use crate::{Format, lines};

/// A fenced code block that holds a diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramBlock {
    /// The format named by the info string.
    pub format: Format,
    /// Byte range of the block body, between the fence lines.
    pub body: Range<usize>,
}

/// Find the diagram fences of a Markdown document.
#[must_use]
pub fn find_markdown_blocks(markdown: &str) -> Vec<DiagramBlock> {
    let mut blocks = Vec::new();
    let mut open: Option<(char, usize, Option<Format>, usize)> = None;
    for source_line in lines(markdown) {
        let (start, offset) = (source_line.start, source_line.end);
        let trimmed = source_line.text.trim_start_matches([' ', '\t']);
        let fence_char = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'));
        let run = fence_char.map_or(0, |c| trimmed.chars().take_while(|&x| x == c).count());
        match open {
            None => {
                if let Some(c) = fence_char.filter(|_| run >= 3) {
                    let info = trimmed[run..].trim();
                    let lang = info.split_whitespace().next().unwrap_or("");
                    open = Some((c, run, Format::from_lang(lang), offset));
                }
            }
            Some((c, len, format, body_start)) => {
                let only_fence = trimmed[run..].trim().is_empty();
                if fence_char == Some(c) && run >= len && only_fence {
                    if let Some(format) = format {
                        blocks.push(DiagramBlock {
                            format,
                            body: body_start..start,
                        });
                    }
                    open = None;
                }
            }
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_body_between_fences() {
        let md = "# T\n\n```mermaid\ngraph TD\n  A-->B\n```\n\ntext\n";
        let blocks = find_markdown_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(&md[blocks[0].body.clone()], "graph TD\n  A-->B\n");
    }

    #[test]
    fn ignores_other_languages_and_nested_shorter_fences() {
        let md = "````text\n```mermaid\nx\n```\n````\n~~~mermaid\nA\n~~~\n";
        let blocks = find_markdown_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(&md[blocks[0].body.clone()], "A\n");
    }

    #[test]
    fn unterminated_fence_yields_nothing() {
        assert_eq!(find_markdown_blocks("```mermaid\nA-->B\n"), Vec::new());
    }

    #[test]
    fn handles_crlf_and_lone_cr_with_original_offsets() {
        let md = "x\r\n```mermaid\rA-->B\r```\r\n";
        let blocks = find_markdown_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(&md[blocks[0].body.clone()], "A-->B\r");
    }
}
