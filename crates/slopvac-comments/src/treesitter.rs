//! The tree-sitter backend, for grammars without an external scanner.

use std::borrow::Cow;
use std::ops::ControlFlow;

use tree_sitter::{Node, ParseOptions, Parser};

use crate::extract::{
    ExtractError, Extraction, Limits, Piece, Stats, finish, push_comment, trimmed,
};
use crate::language::Language;

/// The most bytes one read hands to the parser.
const READ_CHUNK: usize = 4096;

/// A step counter with a ceiling.
struct Meter {
    steps: usize,
    max: usize,
}

impl Meter {
    fn step(&mut self) -> Result<(), ExtractError> {
        self.steps += 1;
        if self.steps > self.max {
            return Err(ExtractError::WalkBudget { budget: self.max });
        }
        Ok(())
    }
}

/// Extract with the language's tree-sitter grammar.
pub(crate) fn extract(
    language: Language,
    source: &str,
    limits: &Limits,
) -> Result<Extraction, ExtractError> {
    let (tree, parse_checks, read_bytes) = parse(language, source, limits)?;
    let syntax = language.syntax();
    let mut meter = Meter {
        steps: 0,
        max: limits.max_walk_steps.unwrap_or(1024 + 8 * source.len()),
    };
    let mut pieces: Vec<Piece<'_>> = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        meter.step()?;
        let node = cursor.node();
        let descend = if is_comment(&node) {
            // Some grammars put the line terminator inside a line comment.
            let range = trimmed(source, node.byte_range());
            if source.get(range.clone()).is_some() {
                push_comment(source, range, syntax, &mut pieces);
            }
            false
        } else {
            true
        };
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
    Ok(finish(
        source,
        pieces,
        syntax,
        Stats {
            parse_checks,
            read_bytes,
            walk_steps: meter.steps,
        },
    ))
}

/// Parse `source` under the parse and read budgets. Returns the tree and the work used.
fn parse(
    language: Language,
    source: &str,
    limits: &Limits,
) -> Result<(tree_sitter::Tree, usize, usize), ExtractError> {
    let grammar = language.grammar().ok_or(ExtractError::Parse {
        language: language.name(),
    })?;
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|error| ExtractError::Grammar {
            language: language.name(),
            reason: error.to_string(),
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
    // Hand the lexer small chunks and charge each one. tree-sitter lexes before it checks
    // progress, so a lexer that rescans the rest of the file would otherwise go uncharged.
    let read_budget = limits
        .max_read_bytes
        .unwrap_or(64 * source.len() + 64 * 1024);
    let mut read_bytes = 0;
    let mut read_exceeded = false;
    let mut read = |offset: usize, _: tree_sitter::Point| -> &[u8] {
        // End the chunk on a code point boundary: some scanners misread a split character.
        let mut end = (offset + READ_CHUNK).min(bytes.len());
        while end > offset && !source.is_char_boundary(end) {
            end -= 1;
        }
        let chunk = bytes.get(offset..end).unwrap_or_default();
        read_bytes += chunk.len();
        if read_bytes > read_budget {
            read_exceeded = true;
            return &[];
        }
        chunk
    };
    let tree = parser.parse_with_options(
        &mut read,
        None,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    );
    if read_exceeded {
        return Err(ExtractError::ReadBudget {
            language: language.name(),
            budget: read_budget,
        });
    }
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
    Ok((tree, parse_checks, read_bytes))
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

fn is_comment(node: &Node<'_>) -> bool {
    let kind = node.kind();
    // Grammars name the node `comment`, `line_comment`, `block_comment`, ... Perl uses
    // `comments`, Haskell `haddock`, F# `xml_doc` for `///` lines, and JavaScript,
    // Rust and Kotlin give a shebang line a node of its own.
    kind.ends_with("comment")
        || matches!(kind, "comments" | "haddock" | "xml_doc" | "hash_bang_line")
        || kind.contains("shebang")
}
