//! The tree walk: find comment nodes, strip them, group line comments into fragments.

use std::ops::Range;

use thiserror::Error;
use tree_sitter::{LanguageError, Node, Parser};

use crate::docstring;
use crate::language::Language;
use crate::span::{Projection, Span, TokenKind};
use crate::strip::{self, Decorated};
use crate::{CommentKind, Fragment};

/// Extraction failed before any comment was read.
#[derive(Debug, Error)]
pub enum ExtractError {
    /// The grammar was built for another tree-sitter ABI than the one linked.
    #[error("the {language} grammar cannot be loaded: {source}")]
    Grammar {
        /// The language whose grammar failed.
        language: &'static str,
        /// The tree-sitter error.
        source: LanguageError,
    },
    /// tree-sitter returned no tree.
    #[error("tree-sitter produced no syntax tree for the {language} source")]
    Parse {
        /// The language that was parsed.
        language: &'static str,
    },
}

/// Extract the comment and docstring prose of `source`.
///
/// Fragments come in source order. Source that does not parse cleanly still yields
/// the comments tree-sitter recognised. The call parses once and walks the tree once, so
/// its cost is linear in the size of the source.
pub fn extract(language: Language, source: &str) -> Result<Vec<Fragment>, ExtractError> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.grammar())
        .map_err(|source| ExtractError::Grammar {
            language: language.name(),
            source,
        })?;
    let tree = parser.parse(source, None).ok_or(ExtractError::Parse {
        language: language.name(),
    })?;

    let syntax = language.syntax();
    let mut pieces = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let mut descend = true;
        if is_comment(&node) {
            descend = false;
            // Some grammars put the line terminator inside a line comment.
            let range = trimmed(source, node.byte_range());
            if source.get(range.clone()).is_some() {
                let decorated = strip::comment(source, range.start, range.end, syntax);
                pieces.push(Piece::new(source, &range, decorated));
            }
        } else if let Some(range) = docstring::range(language, &node, source) {
            descend = false;
            if let Some(decorated) = strip::string_literal(source, range.start, range.end) {
                pieces.push(Piece::new(source, &range, decorated));
            }
        }
        if descend && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    Ok(group(source, pieces))
}

/// `range` without trailing blanks and line terminators.
fn trimmed(source: &str, range: Range<usize>) -> Range<usize> {
    let bytes = source.as_bytes();
    let mut end = range.end.min(bytes.len());
    while end > range.start && matches!(bytes[end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        end -= 1;
    }
    range.start..end
}

fn is_comment(node: &Node<'_>) -> bool {
    let kind = node.kind();
    // Grammars name the node `comment`, `line_comment`, `block_comment`, ... Perl uses
    // `comments`, Haskell `haddock` and F# `xml_doc` for `///` lines.
    kind.ends_with("comment") || matches!(kind, "comments" | "haddock" | "xml_doc")
}

/// A stripped comment with its place in the source.
struct Piece<'a> {
    decorated: Decorated<'a>,
    start: usize,
    /// End of the node.
    end: usize,
    /// Nothing but blanks precedes the comment on its line.
    own_line: bool,
}

impl<'a> Piece<'a> {
    fn new(source: &str, range: &Range<usize>, decorated: Decorated<'a>) -> Self {
        let bytes = source.as_bytes();
        let end = range.end;
        let mut before = range.start;
        while before > 0 && matches!(bytes[before - 1], b' ' | b'\t') {
            before -= 1;
        }
        let own_line = before == 0 || matches!(bytes[before - 1], b'\n' | b'\r');
        Self {
            decorated,
            start: range.start,
            end,
            own_line,
        }
    }
}

/// Whether only blanks and exactly one line terminator lie between two comments.
fn adjacent(source: &str, from: usize, to: usize) -> bool {
    let Some(gap) = source.as_bytes().get(from..to) else {
        return false;
    };
    let mut terminators = 0;
    let mut at = 0;
    while at < gap.len() {
        match gap[at] {
            b' ' | b'\t' => {}
            b'\n' => terminators += 1,
            b'\r' => {
                terminators += 1;
                if gap.get(at + 1) == Some(&b'\n') {
                    at += 1;
                }
            }
            _ => return false,
        }
        at += 1;
    }
    terminators == 1
}

/// A run of comment pieces that read as one text.
struct Group<'a> {
    kind: CommentKind,
    marker: &'a str,
    groupable: bool,
    own_line: bool,
    start: usize,
    end: usize,
    lines: Vec<Range<usize>>,
}

fn group(source: &str, mut pieces: Vec<Piece<'_>>) -> Vec<Fragment> {
    pieces.sort_by_key(|piece| piece.start);
    let mut fragments = Vec::new();
    let mut current: Option<Group<'_>> = None;
    for piece in pieces {
        if let Some(open) = current.as_mut() {
            if piece.start < open.end {
                continue;
            }
            if open.groupable
                && open.own_line
                && piece.decorated.groupable
                && open.kind == piece.decorated.kind
                && open.marker == piece.decorated.marker
                && adjacent(source, open.end, piece.start)
            {
                open.end = piece.end;
                open.lines.extend(piece.decorated.lines);
                continue;
            }
        }
        if let Some(done) = current.take() {
            paragraphs(source, &done, &mut fragments);
        }
        current = Some(Group {
            kind: piece.decorated.kind,
            marker: piece.decorated.marker,
            groupable: piece.decorated.groupable,
            own_line: piece.own_line,
            start: piece.start,
            end: piece.end,
            lines: piece.decorated.lines,
        });
    }
    if let Some(done) = current {
        paragraphs(source, &done, &mut fragments);
    }
    fragments
}

/// Cut a group at its blank lines and join the lines of each paragraph with a wrap.
fn paragraphs(source: &str, group: &Group<'_>, out: &mut Vec<Fragment>) {
    let comment = Span::new(group.start, group.end).unwrap_or_else(|_| Span::empty(group.start));
    let mut projection = Projection::new();
    let mut previous_end = 0;
    for line in &group.lines {
        let Some(text) = source.get(line.clone()).filter(|text| !text.is_empty()) else {
            if !projection.tokens().is_empty() {
                out.push(Fragment::new(
                    group.kind,
                    comment,
                    std::mem::take(&mut projection),
                ));
            }
            continue;
        };
        if !projection.tokens().is_empty() {
            projection.push(TokenKind::Wrap, " ", Span::empty(previous_end));
        }
        let src = Span::new(line.start, line.end).unwrap_or_else(|_| Span::empty(line.start));
        projection.push(TokenKind::Identity, text, src);
        previous_end = line.end;
    }
    if !projection.tokens().is_empty() {
        out.push(Fragment::new(group.kind, comment, projection));
    }
}
