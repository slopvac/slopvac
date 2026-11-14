//! Source ranges that a rule must not report in.
//!
//! A rule names the block kinds it must not look inside (`exclude_blocks`). Inline code is
//! never prose, so it is always skipped. [`skip_ranges`] turns both into source byte
//! ranges. The engine uses them to drop matches, and other crates, such as the term
//! casing check, take them as their skip input.

use std::ops::Range;

use slopvac_model::Span;
use slopvac_parse::{BlockKind, Document};
use slopvac_rules::BlockKind as ExcludeKind;

/// Sorted, merged source ranges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkipRanges {
    ranges: Vec<Span>,
    unsupported: Vec<ExcludeKind>,
}

impl SkipRanges {
    /// The ranges in source order. They do not overlap.
    #[must_use]
    pub fn as_slice(&self) -> &[Span] {
        &self.ranges
    }

    /// The ranges as `Range<usize>`, the input shape of the term checks.
    #[must_use]
    pub fn to_ranges(&self) -> Vec<Range<usize>> {
        self.ranges.iter().map(|s| s.start()..s.end()).collect()
    }

    /// Whether `span` touches a skipped range. An empty span counts when it sits inside one.
    #[must_use]
    pub fn overlaps(&self, span: Span) -> bool {
        let end = span.end().max(span.start() + 1);
        let first = self.ranges.partition_point(|r| r.end() <= span.start());
        self.ranges.get(first).is_some_and(|r| r.start() < end)
    }

    /// Excluded kinds that the parsed document cannot locate, so no range was built for
    /// them. A caller reports these as an incomplete check.
    #[must_use]
    pub fn unsupported(&self) -> &[ExcludeKind] {
        &self.unsupported
    }
}

/// The source ranges a rule with these `exclude_blocks` skips in `doc`: the excluded
/// blocks and every inline code span.
///
/// `front_matter` is the span of the front matter, when the file has one. The parser reads
/// only the body, so it cannot tell. A `list-item` or `quote` exclusion is not located: the
/// parser keeps neither as a block. [`SkipRanges::unsupported`] lists them.
#[must_use]
pub fn skip_ranges(
    doc: &Document,
    exclude: &[ExcludeKind],
    front_matter: Option<Span>,
) -> SkipRanges {
    build(doc, exclude, front_matter, true)
}

/// Like [`skip_ranges`], with inline code skipped only when the caller asks. A `raw` rule
/// reads code on purpose.
pub(crate) fn build(
    doc: &Document,
    exclude: &[ExcludeKind],
    front_matter: Option<Span>,
    inline_code: bool,
) -> SkipRanges {
    let mut ranges: Vec<Span> = Vec::new();
    let mut unsupported = Vec::new();
    if inline_code {
        ranges.extend(doc.code_spans.iter().copied());
    }
    for kind in exclude {
        match kind {
            ExcludeKind::Heading => {
                ranges.extend(blocks(doc, |k| matches!(k, BlockKind::Heading { .. })))
            }
            ExcludeKind::Code => {
                ranges.extend(blocks(doc, |k| matches!(k, BlockKind::CodeBlock { .. })));
            }
            ExcludeKind::Table => ranges.extend(blocks(doc, |k| matches!(k, BlockKind::Table))),
            ExcludeKind::CodeSpan => ranges.extend(doc.code_spans.iter().copied()),
            ExcludeKind::FrontMatter => ranges.extend(front_matter),
            ExcludeKind::ListItem | ExcludeKind::Quote => unsupported.push(*kind),
        }
    }
    SkipRanges {
        ranges: merge(ranges),
        unsupported,
    }
}

fn blocks(doc: &Document, wanted: impl Fn(&BlockKind) -> bool) -> Vec<Span> {
    doc.blocks
        .iter()
        .filter(|b| wanted(&b.kind))
        .map(|b| b.span)
        .collect()
}

fn merge(mut ranges: Vec<Span>) -> Vec<Span> {
    ranges.retain(|r| !r.is_empty());
    ranges.sort();
    let mut out: Vec<Span> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match out.last_mut() {
            Some(last) if range.start() <= last.end() => {
                *last = Span::new(last.start(), last.end().max(range.end()))
                    .expect("start does not exceed end");
            }
            _ => out.push(range),
        }
    }
    out
}
