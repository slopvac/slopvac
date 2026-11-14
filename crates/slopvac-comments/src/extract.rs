//! The tree walk: find comment nodes, strip them, group line comments into fragments.

use std::borrow::Cow;
use std::ops::{ControlFlow, Range};

use thiserror::Error;
use tree_sitter::{LanguageError, Node, ParseOptions, Parser};

use crate::docstring::Finder;
use crate::language::Language;
use crate::span::{Projection, Span, TokenKind};
use crate::strip::{self, Decorated};
use crate::{CommentKind, Fragment};

/// Extraction failed before it produced fragments.
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
    /// The build does not include the grammar. Enable the language's cargo feature.
    #[error("the {language} grammar is not in this build; enable the `{feature}` feature")]
    Disabled {
        /// The language that was requested.
        language: &'static str,
        /// The cargo feature that adds it.
        feature: &'static str,
    },
    /// tree-sitter returned no tree.
    #[error("tree-sitter produced no syntax tree for the {language} source")]
    Parse {
        /// The language that was parsed.
        language: &'static str,
    },
    /// The source is larger than [`Limits::max_bytes`].
    #[error("the source is {len} bytes, over the {max} byte limit")]
    TooLarge {
        /// Size of the source.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// Parsing used more work than the budget allows, so the file is incomplete.
    #[error("parsing the {language} source passed its budget of {budget} progress checks")]
    ParseBudget {
        /// The language that was parsed.
        language: &'static str,
        /// The budget, in tree-sitter progress checks (one per 100 parser operations).
        budget: usize,
    },
    /// Walking the syntax tree used more steps than the budget allows.
    #[error("walking the syntax tree passed its budget of {budget} steps")]
    WalkBudget {
        /// The budget, in node visits.
        budget: usize,
    },
}

/// Bounds on one extraction. A bound that is hit returns an error, so the caller reports
/// the file as incomplete. They count work, not time, so a given input always passes or
/// always fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The largest source accepted, in bytes.
    pub max_bytes: usize,
    /// The most tree-sitter progress checks parsing may use (one per 100 parser
    /// operations). `None` allows `256 + len / 4`. Source files in this repository use
    /// at most `len / 60`, so the default leaves a wide margin and still stops a
    /// quadratic parse, such as a long run of unterminated `/*` in C, early.
    pub max_parse_checks: Option<usize>,
    /// The most node visits the walk may use. `None` allows `1024 + 8 * len`.
    pub max_walk_steps: Option<usize>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_parse_checks: None,
            max_walk_steps: None,
        }
    }
}

/// How much work an extraction used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Progress checks tree-sitter made while parsing.
    pub parse_checks: usize,
    /// Node visits and child inspections in the walk.
    pub walk_steps: usize,
}

/// The fragments of one source and the work it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    /// Fragments in source order.
    pub fragments: Vec<Fragment>,
    /// Work used.
    pub stats: Stats,
}

/// A step counter with a ceiling.
pub(crate) struct Meter {
    steps: usize,
    max: usize,
}

impl Meter {
    pub(crate) fn step(&mut self) -> Result<(), ExtractError> {
        self.steps += 1;
        if self.steps > self.max {
            return Err(ExtractError::WalkBudget { budget: self.max });
        }
        Ok(())
    }
}

/// Extract the comment and docstring prose of `source` within the default [`Limits`].
///
/// Fragments come in source order. Source that does not parse cleanly still yields
/// the comments tree-sitter recognised.
pub fn extract(language: Language, source: &str) -> Result<Vec<Fragment>, ExtractError> {
    extract_with(language, source, &Limits::default()).map(|extraction| extraction.fragments)
}

/// Extract with explicit [`Limits`] and report the work used.
///
/// The call parses once and walks the tree once, so its cost is linear in the size of
/// the source.
pub fn extract_with(
    language: Language,
    source: &str,
    limits: &Limits,
) -> Result<Extraction, ExtractError> {
    if source.len() > limits.max_bytes {
        return Err(ExtractError::TooLarge {
            len: source.len(),
            max: limits.max_bytes,
        });
    }
    let grammar = language.grammar().ok_or(ExtractError::Disabled {
        language: language.name(),
        feature: language.feature(),
    })?;
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|source| ExtractError::Grammar {
            language: language.name(),
            source,
        })?;

    // A lone CR ends a line, but several lexers look for LF only and would read the next
    // line as part of the comment. Parse a view with the same byte offsets and LF there.
    let view = line_feed_view(source);
    let bytes: &[u8] = &view;
    let budget = limits.max_parse_checks.unwrap_or(256 + source.len() / 4);
    let mut parse_checks = 0;
    let mut progress = |_: &tree_sitter::ParseState| {
        parse_checks += 1;
        if parse_checks > budget {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let tree = parser.parse_with_options(
        &mut |offset, _| bytes.get(offset..).unwrap_or_default(),
        None,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    );
    let Some(tree) = tree else {
        return Err(if parse_checks > budget {
            ExtractError::ParseBudget {
                language: language.name(),
                budget,
            }
        } else {
            ExtractError::Parse {
                language: language.name(),
            }
        });
    };

    let syntax = language.syntax();
    let mut meter = Meter {
        steps: 0,
        max: limits.max_walk_steps.unwrap_or(1024 + 8 * source.len()),
    };
    let mut finder = Finder::default();
    let mut pieces = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        meter.step()?;
        let node = cursor.node();
        let mut descend = true;
        if is_comment(&node) {
            descend = false;
            // Some grammars put the line terminator inside a line comment.
            let range = trimmed(source, node.byte_range());
            if source.get(range.clone()).is_some() {
                push_comment(source, range, syntax, &mut pieces);
            }
        } else if let Some(range) = finder.check(language, &node, source, &mut meter)? {
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
    Ok(Extraction {
        fragments: group(source, pieces, !syntax.leaders.is_empty()),
        stats: Stats {
            parse_checks,
            walk_steps: meter.steps,
        },
    })
}

/// `source` with each lone CR replaced by LF. Both are one byte, so offsets hold.
fn line_feed_view(source: &str) -> Cow<'_, [u8]> {
    let bytes = source.as_bytes();
    let lone = |at: usize| bytes[at] == b'\r' && bytes.get(at + 1) != Some(&b'\n');
    if !(0..bytes.len()).any(lone) {
        return Cow::Borrowed(bytes);
    }
    let view = (0..bytes.len())
        .map(|at| if lone(at) { b'\n' } else { bytes[at] })
        .collect();
    Cow::Owned(view)
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
    // `comments`, Haskell `haddock`, F# `xml_doc` for `///` lines, and JavaScript,
    // Rust and Kotlin give a shebang line a node of its own.
    kind.ends_with("comment")
        || matches!(kind, "comments" | "haddock" | "xml_doc" | "hash_bang_line")
        || kind.contains("shebang")
}

/// Strip one comment node into pieces. A line comment node that spans several lines,
/// such as Haskell's or Dart's, becomes one piece per line, so the same grouping rules
/// apply as for comments in separate nodes.
fn push_comment<'a>(
    source: &'a str,
    range: Range<usize>,
    syntax: &crate::language::Syntax,
    pieces: &mut Vec<Piece<'a>>,
) {
    let whole = strip::comment(source, range.start, range.end, syntax);
    let multi_line = source.as_bytes()[range.clone()]
        .iter()
        .any(|&byte| matches!(byte, b'\n' | b'\r'));
    if !(whole.groupable && multi_line) {
        pieces.push(Piece::new(source, &range, whole));
        return;
    }
    for line in strip::split_lines(source.as_bytes(), range.start, range.end) {
        let line = trimmed(source, line);
        let first = line.start
            + source.as_bytes()[line.clone()]
                .iter()
                .take_while(|&&byte| matches!(byte, b' ' | b'\t'))
                .count();
        if first < line.end {
            let decorated = strip::comment(source, first, line.end, syntax);
            pieces.push(Piece::new(source, &(first..line.end), decorated));
        }
    }
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
        let mut before = range.start;
        while before > 0 && matches!(bytes[before - 1], b' ' | b'\t') {
            before -= 1;
        }
        // A BOM is part of the bytes but not of the first line.
        let bom = if source.starts_with('\u{feff}') { 3 } else { 0 };
        let own_line = before <= bom || matches!(bytes[before - 1], b'\n' | b'\r');
        Self {
            decorated,
            start: range.start,
            end: range.end,
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

/// `continues_doc` is set for languages where only the first line of a doc comment has a
/// leader (Haskell's `-- |`), so a plain line comment can continue a doc comment.
fn group(source: &str, mut pieces: Vec<Piece<'_>>, continues_doc: bool) -> Vec<Fragment> {
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
                && (open.kind == piece.decorated.kind
                    || (continues_doc
                        && open.kind == CommentKind::Doc
                        && piece.decorated.kind == CommentKind::Line))
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
