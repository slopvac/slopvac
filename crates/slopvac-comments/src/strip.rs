//! Decoration stripping: the delimiters of a comment or string are gaps, the rest is text.
//!
//! Every function here returns byte ranges into the source. Nothing is copied or decoded,
//! so a range maps to the source by identity. Line terminators are LF, CRLF and a lone CR,
//! as in `docs/contract/positions.md`.

use std::ops::Range;

use crate::CommentKind;
use crate::language::{Block, Syntax};

/// One comment, docstring or string with its delimiters removed.
#[derive(Debug)]
pub(crate) struct Decorated<'a> {
    pub(crate) kind: CommentKind,
    /// The line marker run, for example `//` or `///`. Consecutive line comments group only when the
    /// markers match. Empty for block comments and docstrings.
    pub(crate) marker: &'a str,
    /// Whether this can join the neighbouring line comments.
    pub(crate) groupable: bool,
    /// Text lines in source order, as ranges without terminators or decoration.
    pub(crate) lines: Vec<Range<usize>>,
}

const fn is_blank(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t')
}

fn skip_blank(bytes: &[u8], mut at: usize, end: usize) -> usize {
    while at < end && is_blank(bytes[at]) {
        at += 1;
    }
    at
}

fn trim_blank_end(bytes: &[u8], start: usize, mut end: usize) -> usize {
    while end > start && is_blank(bytes[end - 1]) {
        end -= 1;
    }
    end
}

/// Split `start..end` at LF, CRLF and CR. The terminators are not in the ranges. A range
/// that ends after a terminator yields a final empty line.
pub(crate) fn split_lines(bytes: &[u8], start: usize, end: usize) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut line_start = start;
    let mut at = start;
    while at < end {
        match bytes[at] {
            b'\n' => {
                lines.push(line_start..at);
                at += 1;
                line_start = at;
            }
            b'\r' => {
                lines.push(line_start..at);
                at += 1;
                if at < end && bytes[at] == b'\n' {
                    at += 1;
                }
                line_start = at;
            }
            _ => at += 1,
        }
    }
    lines.push(line_start..end);
    lines
}

/// Strip the indentation, an optional star column and trailing blanks from each line.
fn interior(source: &str, start: usize, end: usize, star_column: bool) -> Vec<Range<usize>> {
    let bytes = source.as_bytes();
    split_lines(bytes, start, end)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let mut at = skip_blank(bytes, line.start, line.end);
            if star_column && index > 0 && at < line.end && bytes[at] == b'*' {
                at = skip_blank(bytes, at + 1, line.end);
            }
            at..trim_blank_end(bytes, at, line.end)
        })
        .collect()
}

/// Strip the delimiters from the comment node at `start..end`.
pub(crate) fn comment<'a>(
    source: &'a str,
    start: usize,
    end: usize,
    syntax: &Syntax,
) -> Decorated<'a> {
    for block in syntax.blocks {
        let found = match *block {
            Block::Fixed {
                open, close, stars, ..
            } => fixed(source, start, end, open, close, stars),
            Block::Lua => lua(source, start, end),
        };
        if let Some(found) = found {
            return found;
        }
    }
    line(source, start, end, syntax)
}

fn block_result(
    source: &str,
    content: Range<usize>,
    star_column: bool,
    doc: bool,
) -> Decorated<'_> {
    Decorated {
        kind: if doc {
            CommentKind::Doc
        } else {
            CommentKind::Block
        },
        marker: "",
        groupable: false,
        lines: interior(source, content.start, content.end, star_column),
    }
}

fn fixed<'a>(
    source: &'a str,
    start: usize,
    end: usize,
    open: &str,
    close: &str,
    star_column: bool,
) -> Option<Decorated<'a>> {
    let text = source.get(start..end)?;
    if !text.starts_with(open) {
        return None;
    }
    let bytes = source.as_bytes();
    let terminated = text.len() >= open.len() + close.len() && text.ends_with(close);
    let mut content_end = if terminated { end - close.len() } else { end };
    let mut content_start = start + open.len();
    let mut doc = false;
    if star_column {
        while content_start < content_end && bytes[content_start] == b'*' {
            content_start += 1;
        }
        let run = &source[start..content_start];
        if run == "/**" {
            doc = true;
        } else if run == "/*" && content_start < content_end && bytes[content_start] == b'!' {
            content_start += 1;
            doc = true;
        }
        if terminated {
            while content_end > content_start && bytes[content_end - 1] == b'*' {
                content_end -= 1;
            }
        }
    }
    Some(block_result(
        source,
        content_start..content_end,
        star_column,
        doc,
    ))
}

fn lua(source: &str, start: usize, end: usize) -> Option<Decorated<'_>> {
    let text = source.get(start..end)?;
    let rest = text.strip_prefix("--[")?;
    let level = rest.bytes().take_while(|&byte| byte == b'=').count();
    if rest.as_bytes().get(level) != Some(&b'[') {
        return None;
    }
    let open_len = 4 + level;
    let close = format!("]{}]", "=".repeat(level));
    let terminated = text.len() >= open_len + close.len() && text.ends_with(&close);
    let content_end = if terminated { end - close.len() } else { end };
    Some(block_result(
        source,
        start + open_len..content_end,
        false,
        false,
    ))
}

fn line<'a>(source: &'a str, start: usize, end: usize, syntax: &Syntax) -> Decorated<'a> {
    let bytes = source.as_bytes();
    let mut marker: &str = "";
    let mut doc = false;
    let lines = split_lines(bytes, start, end)
        .into_iter()
        .enumerate()
        .map(|(index, physical)| {
            let mut at = skip_blank(bytes, physical.start, physical.end);
            let rest = &source[at..physical.end];
            if let Some(found) = syntax
                .line
                .iter()
                .chain(syntax.shebang)
                .find(|m| rest.starts_with(**m))
            {
                let repeated = found.as_bytes()[found.len() - 1];
                let mut after = at + found.len();
                while after < physical.end && bytes[after] == repeated {
                    after += 1;
                }
                if after < physical.end && syntax.line_suffix.contains(&char::from(bytes[after])) {
                    after += 1;
                }
                if index == 0 {
                    marker = &source[at..after];
                    doc = syntax.line_doc.contains(&marker);
                }
                at = after;
            }
            let mut at = skip_blank(bytes, at, physical.end);
            if index == 0 && at < physical.end && syntax.leaders.contains(&char::from(bytes[at])) {
                at = skip_blank(bytes, at + 1, physical.end);
                doc = true;
            }
            at..trim_blank_end(bytes, at, physical.end)
        })
        .collect();
    Decorated {
        kind: if doc {
            CommentKind::Doc
        } else {
            CommentKind::Line
        },
        marker,
        groupable: true,
        lines,
    }
}

/// Whether every physical line of `start..end` begins with a line marker. A grammar that
/// merges a run of line comments into one node (Haskell, Dart) gives such a node. A C
/// comment continued by a backslash does not: its second line has no marker.
pub(crate) fn every_line_has_marker(
    source: &str,
    start: usize,
    end: usize,
    syntax: &Syntax,
) -> bool {
    let bytes = source.as_bytes();
    split_lines(bytes, start, end).into_iter().all(|line| {
        let at = skip_blank(bytes, line.start, line.end);
        source.get(at..line.end).is_some_and(|rest| {
            syntax
                .line
                .iter()
                .chain(syntax.shebang)
                .any(|marker| rest.starts_with(marker))
        })
    })
}
