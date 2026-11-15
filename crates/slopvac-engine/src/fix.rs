//! Safe fixes: apply the single-replacement substitutions a lint run found.
//!
//! [`fix_document`] turns the [`Fix`](slopvac_model::Fix) edits on a document's findings
//! into one atomic edit set ([`slopvac_model::apply_edits`]) and returns the fixed text. It
//! never writes. [`unified_diff`] renders a dry run. The file-level entry point,
//! [`fix_file`](crate::fix_file), lives in `file.rs`.
//!
//! A fix applies only when all of these hold:
//!
//! - The finding exists in the document, so a suppressed finding is never fixed.
//! - The edit covers exactly the text the finding matched in the current source.
//! - The edit touches no front matter, inline code or code block.
//! - No fix accepted earlier in the document overlaps it. Findings run in report order, so
//!   the first one wins and the others stay for the next run.
//! - With a changed-line filter, every line the edit touches is in a changed range.
//!
//! The replacement keeps the case of the text it replaces, as 2.x did: an upper-case match
//! gives an upper-case replacement and a capitalised match gives a capitalised one.
//!
//! The text of an edit is written verbatim, so CRLF line endings and every byte outside the
//! edits stay as they were.
//!
//! Comments in source code are not fixed yet: the engine lints Markdown, MDX, RST and
//! diagrams only (slopvac-8l4.3.14 adds code comments). RST findings are reported with their
//! fixes, but no fix is applied: each is refused as protected until the RST adapter can tell
//! code and literals apart for this step.

use std::ops::RangeInclusive;

use frontmatter_spans::detect;
use similar::TextDiff;
use slopvac_model::{
    Document, Edit, EditError, Finding, LineCol, RuleId, SourceIndex, Span, apply_edits,
};
use slopvac_parse::{
    BlockKind, Document as Parsed, ParseError, mdx_syntax_spans, parse_markdown, parse_mdx,
};

use crate::lint::{EngineError, Input, Kind};
use crate::protect::destinations;

/// Why a finding that carries a fix was left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// The text at the edit is not the text the finding matched: the document changed
    /// since the lint run.
    Stale,
    /// The edit touches front matter, inline code or a code block.
    Protected,
    /// A fix accepted earlier overlaps the edit.
    Overlap,
    /// The edit touches a line outside the changed lines.
    OutsideChangedLines,
}

/// A finding with a fix that was not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The finding's rule.
    pub rule_id: RuleId,
    /// Where the finding starts.
    pub start: LineCol,
    /// Why its fix was refused.
    pub reason: SkipReason,
}

/// The result of fixing one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixReport {
    /// The fixed text. It equals the input when nothing applied.
    pub text: String,
    /// How many findings were fixed.
    pub applied: usize,
    /// The findings with a fix that were left alone, in report order.
    pub skipped: Vec<Skipped>,
}

/// Why a fix run failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FixError {
    /// Linting failed.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The source did not parse.
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// The accepted edits did not apply. This is a bug: they were checked first.
    #[error(transparent)]
    Edit(#[from] EditError),
    /// A file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: String,
        /// What failed.
        source: std::io::Error,
    },
    /// The file is not UTF-8.
    #[error("{0}: not valid UTF-8")]
    NotUtf8(String),
}

/// Fix the findings of `doc`, which was linted from `input`.
///
/// `changed_lines` limits fixes to edits whose every line is in one of the one-based
/// inclusive ranges, the shape a diff scope produces. `None` fixes everywhere.
pub fn fix_document(
    input: &Input<'_>,
    doc: &Document,
    changed_lines: Option<&[RangeInclusive<usize>]>,
) -> Result<FixReport, FixError> {
    // RST is not fixed yet: its code and literals are known only to the docutils run that
    // linted it, so every fix is refused as protected rather than guessed.
    let rst = input.kind == Kind::Rst;
    let protected = if rst {
        Vec::new()
    } else {
        protected_spans(input)?
    };
    let index = SourceIndex::new(input.source, input.bom);
    let mut edits: Vec<Edit> = Vec::new();
    let mut applied = 0;
    let mut skipped = Vec::new();
    for finding in &doc.findings {
        let Some(fix) = &finding.fix else { continue };
        let reason = if rst {
            Some(SkipReason::Protected)
        } else {
            refusal(
                finding,
                fix.edits(),
                input.source,
                &index,
                &protected,
                &edits,
                changed_lines,
            )
        };
        if let Some(reason) = reason {
            skipped.push(Skipped {
                rule_id: finding.rule_id.clone(),
                start: finding.start,
                reason,
            });
            continue;
        }
        edits.extend(fix.edits().iter().map(|e| Edit {
            span: e.span,
            text: preserve_case(&input.source[e.span.start()..e.span.end()], &e.text),
        }));
        applied += 1;
    }
    let text = apply_edits(input.source, &edits)?;
    Ok(FixReport {
        text,
        applied,
        skipped,
    })
}

fn refusal(
    finding: &Finding,
    fix: &[Edit],
    source: &str,
    index: &SourceIndex<'_>,
    protected: &[Span],
    accepted: &[Edit],
    changed_lines: Option<&[RangeInclusive<usize>]>,
) -> Option<SkipReason> {
    // A span from another version of the text may end inside a character or past the end.
    if fix.iter().any(|e| {
        e.span.end() > source.len()
            || !source.is_char_boundary(e.span.start())
            || !source.is_char_boundary(e.span.end())
    }) {
        return Some(SkipReason::Stale);
    }
    if let [edit] = fix
        && finding.extent.map(|e| e.span) == Some(edit.span)
        && source[edit.span.start()..edit.span.end()] != finding.matched_text
    {
        return Some(SkipReason::Stale);
    }
    if fix.iter().any(|e| overlaps_any(e.span, protected)) {
        return Some(SkipReason::Protected);
    }
    let taken: Vec<Span> = accepted.iter().map(|e| e.span).collect();
    if fix.iter().any(|e| overlaps_any(e.span, &taken)) {
        return Some(SkipReason::Overlap);
    }
    if let Some(ranges) = changed_lines
        && !fix.iter().all(|e| lines_changed(index, e.span, ranges))
    {
        return Some(SkipReason::OutsideChangedLines);
    }
    None
}

fn overlaps_any(span: Span, others: &[Span]) -> bool {
    others
        .iter()
        .any(|o| span.start() < o.end() && o.start() < span.end())
}

/// Whether every line `span` touches is in `ranges`.
fn lines_changed(index: &SourceIndex<'_>, span: Span, ranges: &[RangeInclusive<usize>]) -> bool {
    let first = index.line_col(span.start());
    let last = index.locate_last_included(span, slopvac_model::ColumnUnit::CodePoint);
    let (Ok(first), Ok(last)) = (first, last) else {
        return false;
    };
    (first.line()..=last.line()).all(|line| ranges.iter().any(|r| r.contains(&line)))
}

/// Front matter, inline code and code blocks: text that is never prose.
fn protected_spans(input: &Input<'_>) -> Result<Vec<Span>, FixError> {
    let mut spans = Vec::new();
    let mut body_start = 0;
    if !matches!(input.kind, Kind::Diagram(_))
        && let Some(front) = detect(input.source)
    {
        body_start = front.body_start;
        if let Ok(span) = Span::new(0, body_start) {
            spans.push(span);
        }
    }
    let body = Span::new(body_start, input.source.len()).expect("body starts inside the file");
    let parsed: Parsed = match input.kind {
        Kind::Markdown => parse_markdown(input.source, body)?,
        Kind::Mdx => parse_mdx(input.source, body)?,
        Kind::Diagram(_) | Kind::Rst => Parsed::default(),
    };
    spans.extend(parsed.code_spans.iter().copied());
    spans.extend(destinations(input.source, body_start));
    if input.kind == Kind::Mdx {
        spans.extend(mdx_syntax_spans(input.source, body)?);
    }
    spans.extend(
        parsed
            .blocks
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::CodeBlock { .. }))
            .map(|b| b.span),
    );
    Ok(spans)
}

/// `replacement` in the case of `original`: upper case after an upper-case match, a
/// capital first letter after a capitalised one.
fn preserve_case(original: &str, replacement: &str) -> String {
    let cased = original.chars().any(char::is_uppercase);
    if cased && !original.chars().any(char::is_lowercase) {
        return replacement.to_uppercase();
    }
    if original.chars().next().is_some_and(char::is_uppercase) {
        let mut chars = replacement.chars();
        if let Some(first) = chars.next() {
            return first.to_uppercase().chain(chars).collect();
        }
    }
    replacement.to_owned()
}

/// A unified diff of `before` to `after`, headed `a/<path>` and `b/<path>`, with three
/// lines of context. Empty when the texts are equal.
#[must_use]
pub fn unified_diff(path: &str, before: &str, after: &str) -> String {
    if before == after {
        return String::new();
    }
    TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}
