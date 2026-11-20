//! Source code as a document: the prose of its comments.
//!
//! `slopvac-comments` cuts the comments of a source file into fragments, each with a
//! projection onto the source bytes. This module turns the fragments into a
//! [`slopvac_parse::Document`] of paragraphs, so the rule dispatch reads them like any
//! other prose, and builds the two views of the source that the 2.x comment mode used
//! (`_comment_projection` and `comment_blocks` in the Python reference):
//!
//! - the *masked text*, the source with every byte that is not comment text replaced by a
//!   space and the line terminators kept. It is as long as the source, so an offset in it is
//!   a source offset, and a `raw` rule matches only comment text;
//! - the *comment blocks*, the runs of consecutive lines that carry comment text, which are
//!   the unit an inline directive covers.

use std::collections::HashSet;
use std::ops::Range;

use slopvac_comments::{Fragment, Language, TokenKind as CommentToken};
use slopvac_model::Span;
use slopvac_parse::{Block, BlockKind, Document, Projection, TokenKind, parse_markdown};

use crate::directives::{Directives, Lines, Target, scan_lines};
use crate::lint::EngineError;

/// What a source file reads as.
pub(crate) struct CommentText {
    /// One paragraph block for each fragment.
    pub(crate) parsed: Document,
    /// The source with everything but comment text blanked. Directive text is kept: this is
    /// what the directive scan reads.
    pub(crate) masked: String,
    /// [`CommentText::masked`] with the `<!-- ... -->` runs blanked too: what the rules that
    /// read the source text match, so a rule never matches the id in a directive.
    pub(crate) lexical: String,
}

/// Extract the comments of `source` within the default work limits.
///
/// # Errors
/// Returns [`EngineError::Comments`] when a limit is hit: the file is then incomplete, not
/// clean.
pub(crate) fn read(source: &str, language: Language) -> Result<CommentText, EngineError> {
    let fragments = slopvac_comments::extract(language, source)
        .map_err(|e| EngineError::Comments(e.to_string()))?;
    let mut parsed = Document::default();
    let mut masked = source.as_bytes().to_vec();
    let mut kept = 0;
    for fragment in &fragments {
        for token in fragment.projection().tokens() {
            if token.kind == CommentToken::Identity {
                blank(&mut masked, kept, token.src.start());
                kept = token.src.end();
            }
        }
        parsed.blocks.push(block(fragment)?);
    }
    blank(&mut masked, kept, source.len());
    let masked = String::from_utf8(masked)
        .map_err(|e| EngineError::Internal(format!("masked comment text: {e}")))?;
    let lexical = blank_html_comments(&masked);
    Ok(CommentText {
        parsed,
        masked,
        lexical,
    })
}

/// The `<!-- ... -->` runs of one line, as byte ranges. A run that does not close on its line
/// is not one.
fn html_runs(line: &str) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut at = 0;
    while let Some(open) = line[at..].find("<!--") {
        let open = at + open;
        let Some(close) = line[open + 4..].find("-->") else {
            break;
        };
        let end = open + 4 + close + 3;
        runs.push(open..end);
        at = end;
    }
    runs
}

/// `masked` with every `<!-- ... -->` run replaced by spaces. It is as long as `masked`.
fn blank_html_comments(masked: &str) -> String {
    let mut out = String::with_capacity(masked.len());
    let mut line_start = 0;
    let bytes = masked.as_bytes();
    let mut at = 0;
    while at <= bytes.len() {
        if at == bytes.len() || matches!(bytes[at], b'\n' | b'\r') {
            let line = &masked[line_start..at];
            let mut last = 0;
            for run in html_runs(line) {
                out.push_str(&line[last..run.start]);
                out.push_str(&" ".repeat(run.len()));
                last = run.end;
            }
            out.push_str(&line[last..]);
            if at < bytes.len() {
                out.push(char::from(bytes[at]));
            }
            line_start = at + 1;
        }
        at += 1;
    }
    out
}

/// Blank the bytes of `from..to` that are not a line terminator.
fn blank(bytes: &mut [u8], from: usize, to: usize) {
    for byte in &mut bytes[from.min(to)..to] {
        if !matches!(*byte, b'\n' | b'\r') {
            *byte = b' ';
        }
    }
}

fn model_span(start: usize, end: usize) -> Result<Span, EngineError> {
    Span::new(start, end).map_err(|e| EngineError::Internal(e.to_string()))
}

/// A fragment as a paragraph. Its tokens keep their source ranges, so a match maps back to
/// the exact bytes; the delimiters of the comment are gaps between the tokens.
fn block(fragment: &Fragment) -> Result<Block, EngineError> {
    let mut projection = Projection::new();
    for token in fragment.projection().tokens() {
        match token.kind {
            CommentToken::Wrap => projection.push(
                TokenKind::Wrap,
                &token.text,
                model_span(token.src.start(), token.src.end())?,
            ),
            // A directive is not prose: its runs are gaps, kept for the directive scan.
            CommentToken::Identity => {
                let mut last = 0;
                let mut keep = |from: usize, to: usize| -> Result<(), EngineError> {
                    if from < to {
                        projection.push(
                            TokenKind::Identity,
                            &token.text[from..to],
                            model_span(token.src.start() + from, token.src.start() + to)?,
                        );
                    }
                    Ok(())
                };
                for run in html_runs(&token.text) {
                    keep(last, run.start)?;
                    last = run.end;
                }
                keep(last, token.text.len())?;
            }
        }
    }
    let text = fragment.text_span();
    Ok(Block {
        kind: BlockKind::Paragraph,
        span: model_span(text.start(), text.end())?,
        projection: Some(projection),
        containers: Vec::new(),
    })
}

/// `line` without its `<!-- ... -->` runs.
fn without_html_comments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find("<!--") {
        let Some(close) = rest[open + 4..].find("-->") else {
            break;
        };
        out.push_str(&rest[..open]);
        rest = &rest[open + 4 + close + 3..];
    }
    out.push_str(rest);
    out
}

/// The first and last line of each comment block of the masked text: a run of lines with
/// text once the HTML comments are removed. A blank line, a code line, an empty comment
/// line and a line that holds only directives end a run.
fn comment_blocks(lines: &Lines<'_>) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    let mut first = None;
    for (number, text) in lines.iter() {
        if without_html_comments(text).trim().is_empty() {
            blocks.extend(first.take().map(|f| (f, number - 1)));
        } else if first.is_none() {
            first = Some(number);
        }
    }
    blocks.extend(first.map(|f| (f, lines.len())));
    blocks
}

/// The lines of a comment block a directive on `annotation` covers: the block that starts on
/// its line, else the block that holds the next line with comment text.
fn targets(blocks: &[(usize, usize)], annotation: usize) -> Vec<usize> {
    blocks
        .iter()
        .find(|&&(first, last)| first == annotation || last > annotation)
        .map_or_else(Vec::new, |&(first, last)| (first..=last).collect())
}

/// The text of each line as its author wrote it, once the decoration is removed: the
/// delimiter and the one blank after it go, and any further indentation stays, as in 2.x
/// (`_comment_text` in the Python reference). A line without comment text is empty.
///
/// The extractor drops all indentation, so it is read back from the source: the blanks right
/// before the first byte of comment text on the line.
fn authored(source: &Lines<'_>, masked: &Lines<'_>) -> String {
    let mut out: Vec<String> = Vec::with_capacity(masked.len());
    for (number, shown) in masked.iter() {
        let Some(at) = shown.find(|c| c != ' ') else {
            out.push(String::new());
            continue;
        };
        let before = source.text(number).get(..at).unwrap_or("");
        let text = before.trim_end_matches([' ', '\t']);
        let indent = &before[text.len()..];
        // A delimiter stands before the blanks and takes one of them. A line that holds
        // comment text only (inside a block comment) keeps all of its indentation.
        let indent = if text.is_empty() {
            indent
        } else {
            indent.get(1..).unwrap_or("")
        };
        out.push(format!("{indent}{}", shown[at..].trim_end()));
    }
    out.join("\n")
}

/// The lines that hold a Markdown code block in the authored comment text: an example, not
/// a directive.
fn quoted_lines(source: &Lines<'_>, masked: &Lines<'_>) -> HashSet<usize> {
    let text = authored(source, masked);
    let Ok(body) = Span::new(0, text.len()) else {
        return HashSet::new();
    };
    let Ok(document) = parse_markdown(&text, body) else {
        return HashSet::new();
    };
    let at = Lines::new(&text);
    let mut quoted = HashSet::new();
    for block in &document.blocks {
        if matches!(block.kind, BlockKind::CodeBlock { .. }) {
            let last = block.span.end().saturating_sub(1).max(block.span.start());
            quoted.extend(at.line_of(block.span.start())..=at.line_of(last));
        }
    }
    quoted
}

/// The directives written in comments, scoped to comment blocks. `masked` is the comment
/// text of `source`, as [`read`] returns it.
pub(crate) fn directives<'a>(
    source: &str,
    masked: &str,
    lookup: impl Fn(&str) -> Target<'a>,
) -> Directives {
    let source = Lines::new(source);
    let lines = Lines::new(masked);
    let blocks = comment_blocks(&lines);
    let quoted = quoted_lines(&source, &lines);
    scan_lines(&lines, &quoted, &|number| targets(&blocks, number), lookup)
}
