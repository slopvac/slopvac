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
//! The text of an edit is the reported replacement, already in the case of the text it
//! replaces (the engine does that when it generates the edit), so a client that applies the
//! reported edits gets the same text as [`fix_document`].
//!
//! The text of an edit is written verbatim, so CRLF line endings and every byte outside the
//! edits stay as they were.
//!
//! # HTML
//!
//! In a standalone HTML file only the content of a text node is fixed. Tags, attributes,
//! comments, the doctype, CDATA, hidden and code elements, the markup inside white space that
//! the parse collapses, and the addresses in the decoded text are protected. An edit may
//! replace a character reference whole but may not cut one. The engine writes the
//! replacement of an HTML edit as text, with `&`, `<` and `>` as character references. A
//! file with text that HTML does not decode (`<xmp>`, `<plaintext>`, `<iframe>`,
//! `<noembed>`, `<noframes>` or CDATA) gets no replacement that needs one.
//!
//! # Source code
//!
//! The comments of a source file are linted but not fixed. Every finding in one is refused as
//! [`SkipReason::Protected`], and the file is returned as it is.
//!
//! # RST
//!
//! 2.x never fixed RST (its `--fix` rewrote Markdown, MDX and text only), so this is a 3.0
//! addition. Only prose that the RST adapter placed word by word is fixed. Everything else
//! is protected, including the code and literal blocks, directive markup and options,
//! comments, link targets and `_target:` definitions, substitution definitions, roles,
//! footnote and substitution references, field names, and any block the adapter could place
//! only by line. Link text stays fixable except where it names the link's target (see
//! [`crate::rst`]). The spans come from a second run of the docutils driver on the current
//! source, so a missing interpreter is a [`FixError::Rst`], never a silent skip. Inside a
//! table a fix must keep the width of the text (ASCII of equal length), because the cells
//! are not re-padded; any other edit there is refused as protected.

use std::collections::BTreeSet;
use std::ops::RangeInclusive;

use frontmatter_spans::detect;
use similar::TextDiff;
use slopvac_model::{
    Document, Edit, EditError, Finding, LineCol, RuleId, SourceIndex, Span, SpanError, apply_edits,
};
use slopvac_parse::{
    BlockKind, Document as Parsed, ParseError, mdx_syntax_spans, parse_markdown, parse_mdx,
};

use crate::lint::{Engine, EngineError, Input, Kind};
use crate::prepare::{html_file, with_html_prose};
use crate::protect::{HtmlProtection, destinations, html_destinations};
use crate::rst::RstError;

/// Why a finding that carries a fix was left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// The text at the edit is not the text the finding matched: the document changed
    /// since the lint run.
    Stale,
    /// The edit touches front matter, inline code, a code block or, in RST, anything but
    /// plain prose words, or in HTML anything but text; it changes the width of text in an
    /// RST table, or it sits in the comments of a source file, which are never edited.
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
    /// An RST file could not be read, so its code and link targets are unknown.
    #[error("{}", crate::messages::fix_rst(.0))]
    Rst(#[from] RstError),
    /// The accepted edits did not apply. This is a bug: they were checked first.
    #[error(transparent)]
    Edit(#[from] EditError),
    /// A protected range of a parsed file ends before it starts. Only a bug can cause it,
    /// because the ranges come from the parse in source order.
    #[error(transparent)]
    Span(#[from] SpanError),
    /// A file could not be read or written.
    #[error("{}", crate::messages::path_error(.path, .source))]
    Io {
        /// The file.
        path: String,
        /// What failed.
        source: std::io::Error,
    },
    /// The file is not UTF-8.
    #[error("{}", crate::messages::not_utf8(.0))]
    NotUtf8(String),
}

/// Fix the findings of `doc`, which was linted from `input` by `engine`.
///
/// `engine` reads RST: its adapter finds the protected spans of the current source.
///
/// `changed_lines` limits fixes to edits whose every line is in one of the one-based
/// inclusive ranges, the shape a diff scope produces. `None` fixes everywhere.
pub fn fix_document(
    engine: &Engine,
    input: &Input<'_>,
    doc: &Document,
    changed_lines: Option<&[RangeInclusive<usize>]>,
) -> Result<FixReport, FixError> {
    let mut tables = Vec::new();
    // Comment text is not edited: every fix in a source file is refused.
    let comments = matches!(input.kind, Kind::Comments(_));
    let fixes = doc.findings.iter().any(|f| f.fix.is_some());
    let html = input.kind == Kind::Html;
    let (protected, atoms) = if comments || (html && !fixes) {
        // Nothing is edited, so an HTML file needs no parse or address scan.
        (Vec::new(), Vec::new())
    } else if input.kind != Kind::Rst {
        protected_spans(input)?
    } else if fixes {
        let parsed = engine.rst_adapter().parse(input.source)?;
        tables = parsed
            .document
            .blocks
            .iter()
            .filter(|b| b.kind == BlockKind::Table)
            .map(|b| b.span)
            .collect();
        (parsed.protected, Vec::new())
    } else {
        (Vec::new(), Vec::new())
    };
    let guard = Guard::new(protected, atoms);
    let mut undecoded: Option<bool> = None;
    let index = SourceIndex::new(input.source, input.bom);
    let mut edits: Vec<Edit> = Vec::new();
    let mut taken: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut applied = 0;
    let mut skipped = Vec::new();
    for finding in &doc.findings {
        let Some(fix) = &finding.fix else { continue };
        if comments {
            skipped.push(Skipped {
                rule_id: finding.rule_id.clone(),
                start: finding.start,
                reason: SkipReason::Protected,
            });
            continue;
        }
        let mut reason = refusal(
            finding,
            fix.edits(),
            input.source,
            &index,
            &guard,
            &taken,
            changed_lines,
        );
        // The engine writes an HTML replacement with character references for `&`, `<` and
        // `>`. Text that HTML does not decode, such as `<xmp>` content or CDATA, would show
        // one as written, so a replacement that holds one is refused in such a file.
        if reason.is_none()
            && html
            && fix.edits().iter().any(|e| e.text.contains(['&', '<', '>']))
            && *undecoded.get_or_insert_with(|| has_undecoded_text(input.source))
        {
            reason = Some(SkipReason::Protected);
        }
        // A table cell keeps the width of its text, or the borders no longer line up.
        if reason.is_none()
            && fix.edits().iter().any(|e| {
                overlaps_any(e.span, &tables)
                    && !same_width(&input.source[e.span.start()..e.span.end()], &e.text)
            })
        {
            reason = Some(SkipReason::Protected);
        }
        if let Some(reason) = reason {
            skipped.push(Skipped {
                rule_id: finding.rule_id.clone(),
                start: finding.start,
                reason,
            });
            continue;
        }
        for edit in fix.edits() {
            taken.insert((edit.span.start(), edit.span.end()));
            edits.push(edit.clone());
        }
        applied += 1;
    }
    let text = apply_edits(input.source, &edits)?;
    Ok(FixReport {
        text,
        applied,
        skipped,
    })
}

/// Whether `new` takes the same columns as `old`. Only ASCII text is known to: another
/// character may be wide or combining, and width tables are not worth a dependency here.
fn same_width(old: &str, new: &str) -> bool {
    old == new || (old.is_ascii() && new.is_ascii() && old.len() == new.len())
}

/// Whether `source` holds an element or section whose text HTML does not decode: `<xmp>`,
/// `<plaintext>`, `<iframe>`, `<noembed>`, `<noframes>` or CDATA. A character reference there
/// shows as written. The check reads the bytes and may find such a tag in a comment or an
/// attribute: it then refuses more than it must, never less.
fn has_undecoded_text(source: &str) -> bool {
    const OPENERS: [&[u8]; 6] = [
        b"<xmp",
        b"<plaintext",
        b"<iframe",
        b"<noembed",
        b"<noframes",
        b"<![cdata[",
    ];
    let bytes = source.as_bytes();
    bytes.iter().enumerate().any(|(at, &b)| {
        b == b'<'
            && OPENERS.iter().any(|opener| {
                bytes
                    .get(at..at + opener.len())
                    .is_some_and(|found| found.eq_ignore_ascii_case(opener))
            })
    })
}

/// The protected ranges of a file, merged and sorted once, so each edit is checked by a
/// binary search.
struct Guard {
    /// Ranges in `(start, end)` order, where no range starts before the one before it ends.
    spans: Vec<Span>,
    /// Ranges an edit may replace whole but may not start or end inside, in the same order.
    atoms: Vec<Span>,
}

impl Guard {
    fn new(mut spans: Vec<Span>, mut atoms: Vec<Span>) -> Self {
        Self {
            spans: merged(&mut spans),
            atoms: merged(&mut atoms),
        }
    }

    /// Whether `span` overlaps a protected range, or starts or ends inside an atom.
    fn refuses(&self, span: Span) -> bool {
        overlaps_sorted(span, &self.spans)
            || [span.start(), span.end()].into_iter().any(|at| {
                let before = self.atoms.partition_point(|a| a.start() < at);
                before
                    .checked_sub(1)
                    .and_then(|i| self.atoms.get(i))
                    .is_some_and(|a| at < a.end())
            })
    }
}

/// `spans` sorted, with each range that overlaps the one before it merged into it. Ranges
/// that only touch stay apart, so an insertion between them is still allowed.
fn merged(spans: &mut [Span]) -> Vec<Span> {
    spans.sort_unstable_by_key(|s| (s.start(), s.end()));
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for &span in spans.iter() {
        match out.last_mut() {
            Some(last) if span.start() < last.end() => {
                if span.end() > last.end() {
                    // Both ranges are valid and the first starts no later, so this cannot fail.
                    if let Ok(joined) = Span::new(last.start(), span.end()) {
                        *last = joined;
                    }
                }
            }
            _ => out.push(span),
        }
    }
    out
}

/// Whether `span` overlaps one of `sorted`, ranges in order where none starts before the one
/// before it ends. Their ends then ascend too, so the last range that starts before `span`
/// ends is the only one to check.
fn overlaps_sorted(span: Span, sorted: &[Span]) -> bool {
    let before = sorted.partition_point(|o| o.start() < span.end());
    before
        .checked_sub(1)
        .and_then(|i| sorted.get(i))
        .is_some_and(|o| span.start() < o.end())
}

fn refusal(
    finding: &Finding,
    fix: &[Edit],
    source: &str,
    index: &SourceIndex<'_>,
    guard: &Guard,
    taken: &BTreeSet<(usize, usize)>,
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
    if fix.iter().any(|e| guard.refuses(e.span)) {
        return Some(SkipReason::Protected);
    }
    // Accepted edits never overlap each other, so their ends ascend with their starts, and
    // the last one that starts before an edit ends is the only one that can overlap it.
    if fix.iter().any(|e| {
        taken
            .range(..(e.span.end(), 0))
            .next_back()
            .is_some_and(|&(_, end)| e.span.start() < end)
    }) {
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

/// Front matter, inline code and code blocks: text that is never prose. The second list holds
/// the ranges an edit may not cut, the character references of an HTML file.
fn protected_spans(input: &Input<'_>) -> Result<(Vec<Span>, Vec<Span>), FixError> {
    let mut spans = Vec::new();
    let mut atoms = Vec::new();
    let mut body_start = 0;
    // A diagram and a standalone HTML file have no front matter.
    if !matches!(input.kind, Kind::Diagram(_) | Kind::Html)
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
        Kind::Html => html_file(input.source, body)?,
        // Source files never get here: `fix_document` refuses their findings first.
        Kind::Diagram(_) | Kind::Rst | Kind::Comments(_) => Parsed::default(),
    };
    let parsed = with_html_prose(
        input.source,
        &SourceIndex::new(input.source, input.bom),
        parsed,
    )?;
    spans.extend(parsed.code_spans.iter().copied());
    match input.kind {
        Kind::Html => {
            let HtmlProtection {
                spans: html,
                atoms: references,
            } = html_destinations(input.source, &parsed)?;
            spans.extend(html);
            atoms = references;
        }
        Kind::Markdown | Kind::Mdx | Kind::Rst | Kind::Diagram(_) | Kind::Comments(_) => {
            spans.extend(destinations(input.source, body_start));
        }
    }
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
    Ok((spans, atoms))
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
