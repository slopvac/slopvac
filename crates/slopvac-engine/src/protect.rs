//! Spans a fix must not touch because they decide where a link goes.
//!
//! A substitution in a link destination, a reference definition, the label of a resolved
//! reference or an HTML attribute changes what the link points to, so [`destinations`]
//! collects those spans from the parser's own events. Bare URLs, `www.` hosts and email
//! addresses are not links to `CommonMark`, so a text scan adds them. The result applies to
//! rules of every scope, including `raw`, which read the source text.
//!
//! Link text stays fixable: the `delta` in `[delta](https://example.com)` is prose.

use crate::sub;
use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use slopvac_model::Span;

const OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_HEADING_ATTRIBUTES);

/// The protected spans in `source`, reading from `body_start` (the front matter, if any,
/// ends there).
pub(crate) fn destinations(source: &str, body_start: usize) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut push = |range: Range<usize>| {
        if let Ok(span) = Span::new(range.start, range.end) {
            spans.push(span);
        }
    };
    let body = sub(source, body_start..);
    let mut events = Parser::new_ext(body, OPTIONS).into_offset_iter();
    // Open links: where the last child ends, so the rest of the node is the target part.
    let mut open: Vec<(LinkType, usize)> = Vec::new();
    for (event, range) in events.by_ref() {
        let shift = |r: &Range<usize>| r.start + body_start..r.end + body_start;
        match &event {
            Event::Start(Tag::Link { link_type, .. } | Tag::Image { link_type, .. }) => {
                open.push((*link_type, range.start));
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some((link_type, text_end)) = open.pop() {
                    let whole = range.clone();
                    let target = text_end..whole.end;
                    match link_type {
                        // An address is all target.
                        // `[text]` and `[text][]` are named by the label, which is the text.
                        LinkType::Autolink
                        | LinkType::Email
                        | LinkType::Collapsed
                        | LinkType::Shortcut => push(shift(&whole)),
                        LinkType::Inline
                        | LinkType::Reference
                        | LinkType::ReferenceUnknown
                        | LinkType::CollapsedUnknown
                        | LinkType::ShortcutUnknown
                        | LinkType::WikiLink { .. } => push(shift(&target)),
                    }
                }
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                for tag in tags(sub(body, range.clone())) {
                    push(range.start + tag.start + body_start..range.start + tag.end + body_start);
                }
            }
            Event::Start(_)
            | Event::End(_)
            | Event::Text(_)
            | Event::Code(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::FootnoteReference(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::Rule
            | Event::TaskListMarker(_) => {}
        }
        if let Some(top) = open.last_mut()
            && !matches!(event, Event::Start(Tag::Link { .. } | Tag::Image { .. }))
        {
            top.1 = top.1.max(range.end);
        }
    }
    for (_, def) in events.reference_definitions().iter() {
        push(def.span.start + body_start..def.span.end + body_start);
    }
    for run in runs(source) {
        for (from, to) in addresses(sub(source, run.clone())) {
            push(run.start + from..run.start + to);
        }
    }
    spans
}

/// Maximal runs of non-space text.
fn runs(source: &str) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start = None;
    for (at, c) in source
        .char_indices()
        .chain(std::iter::once((source.len(), ' ')))
    {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(at),
            (true, Some(from)) => {
                runs.push(from..at);
                start = None;
            }
            _ => {}
        }
    }
    runs
}

/// Byte ranges of the tags in `html`, from `<` to the closing `>` with quoted values
/// skipped over, so an attribute value is inside the range.
fn tags(html: &str) -> Vec<Range<usize>> {
    let bytes = html.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while let Some(&current) = bytes.get(i) {
        let opens = current == b'<'
            && bytes
                .get(i + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'/' || *b == b'!');
        if !opens {
            i += 1;
            continue;
        }
        let mut quote = None;
        let mut j = i + 1;
        while let Some(&byte) = bytes.get(j) {
            match (quote, byte) {
                (None, b'>') => break,
                (None, q @ (b'"' | b'\'')) => quote = Some(q),
                (Some(q), b) if b == q => quote = None,
                _ => {}
            }
            j += 1;
        }
        let end = (j + 1).min(bytes.len());
        found.push(i..end);
        i = end;
    }
    found
}

/// Byte ranges in one run of non-space text that are an address: a URL from its scheme to
/// the end of the run, a `www.` host from its start to the end of the run, an email
/// address, or an angle-bracket autolink such as `<tel:123>`.
fn addresses(run: &str) -> Vec<(usize, usize)> {
    const STOP: &[char] = &['[', ']', '(', ')', '<', '>', '"', '\'', ',', ';'];
    let mut found = Vec::new();
    for (at, _) in run.match_indices("://") {
        let scheme = sub(run, ..at)
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || "+.-".contains(c)))
            .map_or(0, |i| i + 1);
        found.push((scheme, run.len()));
    }
    for (at, _) in run.match_indices("www.") {
        if sub(run, ..at)
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric())
        {
            found.push((at, run.len()));
        }
    }
    for (at, _) in run.match_indices('@') {
        let start = sub(run, ..at).rfind(STOP).map_or(0, |i| i + 1);
        let end = sub(run, at..).find(STOP).map_or(run.len(), |i| at + i);
        found.push((start, end));
    }
    for (at, _) in run.match_indices('<') {
        let rest = sub(run, at + 1..);
        let scheme = rest.find(':').filter(|&i| {
            i > 1
                && sub(rest, ..i).starts_with(|c: char| c.is_ascii_alphabetic())
                && sub(rest, ..i)
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        });
        if let (Some(_), Some(close)) = (scheme, rest.find('>')) {
            found.push((at, at + 1 + close + 1));
        }
    }
    found
}
