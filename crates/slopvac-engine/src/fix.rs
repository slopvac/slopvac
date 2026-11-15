//! Safe fixes: apply the single-replacement substitutions a lint run found.
//!
//! [`fix_document`] turns the [`Fix`](slopvac_model::Fix) edits on a document's findings
//! into one atomic edit set ([`slopvac_model::apply_edits`]) and returns the fixed text. It
//! never writes. [`fix_file`] reads a file, lints it, fixes it, optionally writes the
//! result back, and lints the fixed text again. [`unified_diff`] renders a dry run.
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
//! Comments in source code are not fixed yet: the engine lints Markdown, MDX and diagrams
//! only (slopvac-8l4.3.14 adds code comments).

use std::ops::RangeInclusive;
use std::path::Path;

use frontmatter_spans::detect;
use similar::TextDiff;
use slopvac_model::{
    Document, Edit, EditError, Finding, LineCol, RuleId, SourceIndex, Span, apply_edits,
};
use slopvac_parse::{BlockKind, Document as Parsed, ParseError, parse_markdown, parse_mdx};

use crate::lint::{Engine, EngineError, Input, Kind};

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

/// Why [`fix_file`] left a file unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// The path is a symbolic link. Writing would change the target.
    Symlink,
    /// The file has more than one hard link. Writing would change every name.
    Hardlinked,
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
    let protected = protected_spans(input)?;
    let index = SourceIndex::new(input.source, input.bom);
    let mut edits: Vec<Edit> = Vec::new();
    let mut applied = 0;
    let mut skipped = Vec::new();
    for finding in &doc.findings {
        let Some(fix) = &finding.fix else { continue };
        let reason = refusal(
            finding,
            fix.edits(),
            input.source,
            &index,
            &protected,
            &edits,
            changed_lines,
        );
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
    if fix.iter().any(|e| e.span.end() > source.len()) {
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
        Kind::Diagram(_) => Parsed::default(),
    };
    spans.extend(parsed.code_spans.iter().copied());
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

/// What [`fix_file`] did to one file.
#[derive(Debug, Clone)]
pub struct FileFix {
    /// The lint report: of the fixed text when the file was written, else of the file as
    /// it was.
    pub document: Document,
    /// The fix result. Empty and unchanged when the file was refused.
    pub report: FixReport,
    /// The unified diff of the change, empty when nothing changed or the file was refused.
    pub diff: String,
    /// Whether the file was written.
    pub written: bool,
    /// Why the file was left alone, if it was.
    pub refused: Option<Refusal>,
}

/// Lint `path`, fix it and, unless `dry_run`, write the result in place.
///
/// A symbolic link or a file with several hard links is refused and left unchanged. The
/// lint report of a written file is rerun on the fixed text, so a caller scores what is on
/// disk. A dry run changes nothing and reports the diff the write would make.
pub fn fix_file(
    engine: &Engine,
    path: &Path,
    kind: Kind,
    dry_run: bool,
    changed_lines: Option<&[RangeInclusive<usize>]>,
) -> Result<FileFix, FixError> {
    let shown = path.to_string_lossy();
    let io = |source| FixError::Io {
        path: shown.to_string(),
        source,
    };
    let bytes = std::fs::read(path).map_err(io)?;
    let source = String::from_utf8(bytes).map_err(|_| FixError::NotUtf8(shown.to_string()))?;
    let input = Input {
        kind,
        ..Input::markdown(&shown, &source)
    };
    let document = engine.lint(&input)?;
    let refused = refuse_path(path).map_err(io)?;
    if refused.is_some() {
        return Ok(FileFix {
            document,
            report: FixReport {
                text: source,
                applied: 0,
                skipped: Vec::new(),
            },
            diff: String::new(),
            written: false,
            refused,
        });
    }
    let report = fix_document(&input, &document, changed_lines)?;
    let diff = unified_diff(&shown, &source, &report.text);
    if dry_run || report.text == source {
        return Ok(FileFix {
            document,
            report,
            diff,
            written: false,
            refused: None,
        });
    }
    std::fs::write(path, report.text.as_bytes()).map_err(io)?;
    let relinted = Input {
        source: &report.text,
        ..input
    };
    let document = engine.lint(&relinted)?;
    Ok(FileFix {
        document,
        report,
        diff,
        written: true,
        refused: None,
    })
}

fn refuse_path(path: &Path) -> std::io::Result<Option<Refusal>> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(Some(Refusal::Symlink));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() > 1 {
            return Ok(Some(Refusal::Hardlinked));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::preserve_case;

    #[test]
    fn replacement_follows_the_case_of_the_match() {
        for (original, replacement, want) in [
            ("utilize", "use", "use"),
            ("Utilize", "use", "Use"),
            ("UTILIZE", "use", "USE"),
            ("In order to", "to", "To"),
            ("iPhone", "phone", "phone"),
            ("Ünder", "over", "Over"),
        ] {
            assert_eq!(preserve_case(original, replacement), want, "{original}");
        }
    }
}
