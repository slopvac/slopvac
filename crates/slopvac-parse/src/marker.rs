//! Standalone marker lines in Markdown, outside code.

use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag};

/// A line that holds only a marker, found outside every code block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerLine<'m> {
    /// The marker the line holds, as passed to [`marker_lines`].
    pub marker: &'m str,
    /// Byte offset of the line start, after a leading byte-order mark.
    pub start: usize,
    /// Byte offset just past the line terminator.
    pub end: usize,
}

/// Lines of `text` that consist of one of `markers` and nothing else (surrounding
/// whitespace and a leading byte-order mark aside), in source order.
///
/// Lines inside fenced or indented code blocks are skipped: they show a marker as an
/// example. A terminator is `\n`, `\r\n` or a lone `\r`. Code blocks come from the
/// `CommonMark` parser, so every caller agrees on what counts as code.
#[must_use]
pub fn marker_lines<'m>(text: &str, markers: &[&'m str]) -> Vec<MarkerLine<'m>> {
    let code: Vec<Range<usize>> = Parser::new(text)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(event, Event::Start(Tag::CodeBlock(_))).then_some(range)
        })
        .collect();
    let mut cursor = 0;
    let mut found = Vec::new();
    for span in line_spans(text) {
        // Code block ranges and line starts both ascend, so one pass suffices.
        while code.get(cursor).is_some_and(|c| c.end <= span.start) {
            cursor += 1;
        }
        // An indented block's range starts after its indent, so test for overlap.
        if code.get(cursor).is_some_and(|c| c.start < span.end) {
            continue;
        }
        let Some(line) = text.get(span.clone()) else {
            continue;
        };
        let value = line.trim().trim_start_matches('\u{feff}');
        if let Some(marker) = markers.iter().find(|m| **m == value) {
            let bom = if line.starts_with('\u{feff}') {
                '\u{feff}'.len_utf8()
            } else {
                0
            };
            found.push(MarkerLine {
                marker,
                start: span.start + bom,
                end: span.end,
            });
        }
    }
    found
}

/// Byte ranges of every line, terminator included.
fn line_spans(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                spans.push(start..i + 1);
                start = i + 1;
            }
            b'\r' => {
                let end = if bytes.get(i + 1) == Some(&b'\n') {
                    i + 2
                } else {
                    i + 1
                };
                spans.push(start..end);
                start = end;
                i = end - 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        spans.push(start..bytes.len());
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &str = "<!-- m -->";

    fn starts(text: &str) -> Vec<usize> {
        marker_lines(text, &[M]).iter().map(|l| l.start).collect()
    }

    #[test]
    fn finds_a_marker_in_prose() {
        assert_eq!(starts(&format!("a\n\n{M}\n")), vec![3]);
    }

    #[test]
    fn skips_fenced_and_indented_code() {
        assert_eq!(starts(&format!("```\n{M}\n```\n")), Vec::<usize>::new());
        assert_eq!(starts(&format!("para\n\n    {M}\n")), Vec::<usize>::new());
        assert_eq!(starts(&format!("para\n\n\t{M}\n")), Vec::<usize>::new());
    }

    #[test]
    fn handles_every_terminator_and_a_byte_order_mark() {
        // "a\r\n" (3) + "\r\n" (2) puts the BOM line at 5; the marker starts after
        // the 3-byte BOM.
        let text = format!("a\r\n\r\n\u{feff}{M}\r\nb\r\r{M}");
        let lines = marker_lines(&text, &[M]);
        let first = 5 + '\u{feff}'.len_utf8();
        assert_eq!(
            lines,
            vec![
                MarkerLine {
                    marker: M,
                    start: first,
                    end: first + M.len() + 2
                },
                MarkerLine {
                    marker: M,
                    start: text.len() - M.len(),
                    end: text.len()
                },
            ]
        );
    }
}
