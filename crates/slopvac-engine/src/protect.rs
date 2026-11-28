//! Spans a fix must not touch because they decide where a link goes.
//!
//! A substitution in a link destination, a reference definition, the label of a resolved
//! reference or an HTML attribute changes what the link points to, so [`destinations`]
//! collects those spans from the parser's own events. Bare URLs, `www.` hosts and email
//! addresses are not links to `CommonMark`, so a text scan adds them. The result applies to
//! rules of every scope, including `raw`, which read the source text.
//!
//! Link text stays fixable: the `delta` in `[delta](https://example.com)` is prose.
//!
//! A standalone HTML file is not Markdown: [`html_destinations`] protects every byte that
//! is not the content of a text node, and the addresses in that text.

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use slopvac_model::{Span, SpanError};
use slopvac_parse::{BlockKind, Document, Projection, TokenKind};

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
    let body = &source[body_start..];
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
                        _ => push(shift(&target)),
                    }
                }
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                for tag in tags(&body[range.clone()]) {
                    push(range.start + tag.start + body_start..range.start + tag.end + body_start);
                }
            }
            _ => {}
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
        for (from, to) in addresses(&source[run.clone()]) {
            push(run.start + from..run.start + to);
        }
    }
    spans
}

/// What a fix of a standalone HTML file may not touch.
pub(crate) struct HtmlProtection {
    /// Bytes that no edit may cover.
    pub(crate) spans: Vec<Span>,
    /// Character references in the text. An edit that touches one must replace all of it.
    pub(crate) atoms: Vec<Span>,
}

/// The protected spans of a standalone HTML file, read from `doc`, its parse.
///
/// Only the content of a text node is fixable: the characters and character references that
/// the tokenizer read as text of the prose. Every other byte is protected: tags with their
/// attribute names and values, comments, the doctype, CDATA, and the content of hidden
/// elements and code. The parse merges the white space on both sides of a tag into one
/// space, so only the white space before the first byte of markup and after the last one in
/// that range counts as text. Inside the text, bare URLs, `www.` hosts and email addresses
/// are protected too, found in the decoded text so that `&#64;` reads as `@`. The file is not
/// Markdown, so `[label](target)` in its text is prose.
///
/// # Errors
/// Returns a [`SpanError`] if a protected range would end before it starts. The ranges come
/// from the parse in source order, so this is a bug.
pub(crate) fn html_destinations(source: &str, doc: &Document) -> Result<HtmlProtection, SpanError> {
    let mut text: Vec<(usize, usize)> = Vec::new();
    let mut atoms = Vec::new();
    let mut spans = Vec::new();
    let prose = doc
        .blocks
        .iter()
        .filter(|b| !matches!(b.kind, BlockKind::CodeBlock { .. }))
        .filter_map(|b| b.projection.as_ref());
    for projection in prose {
        for token in projection.tokens() {
            let src = token.src;
            match token.kind {
                TokenKind::Identity => text.push((src.start(), src.end())),
                TokenKind::Entity => {
                    text.push((src.start(), src.end()));
                    atoms.push(src);
                }
                TokenKind::Collapse => {
                    let (lead, trail) = white_ends(source, src);
                    text.push((src.start(), lead));
                    text.push((trail, src.end()));
                }
                _ => {}
            }
        }
        decoded_addresses(projection, &mut spans)?;
    }
    text.retain(|(start, end)| start < end);
    text.sort_unstable();
    let mut segments: Vec<(usize, usize)> = Vec::new();
    for (start, end) in text {
        match segments.last_mut() {
            Some((_, to)) if start <= *to => *to = (*to).max(end),
            _ => segments.push((start, end)),
        }
    }
    let mut at = 0;
    for (start, end) in segments {
        if start > at {
            spans.push(Span::new(at, start)?);
        }
        at = end;
    }
    if source.len() > at {
        spans.push(Span::new(at, source.len())?);
    }
    atoms.sort_unstable_by_key(|a| (a.start(), a.end()));
    Ok(HtmlProtection { spans, atoms })
}

/// Where the white space at the ends of the collapsed range `span` stops: the end of the
/// leading run and the start of the trailing one. When the range is all white space, the
/// whole of it is text. When it cannot be read, none of it is.
fn white_ends(source: &str, span: Span) -> (usize, usize) {
    let Some(bytes) = source.as_bytes().get(span.start()..span.end()) else {
        return (span.start(), span.end());
    };
    let space = |b: &&u8| matches!(**b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c');
    let lead = bytes.iter().take_while(space).count();
    if lead == bytes.len() {
        return (span.end(), span.start());
    }
    let trail = bytes.iter().rev().take_while(space).count();
    (span.start() + lead, span.end() - trail)
}

/// Push to `out` the source bytes of each address in the decoded text of `projection`.
///
/// A character of an identity token maps to its own byte, and any other token maps whole,
/// so an address keeps each character reference in it. The addresses are merged first, so
/// one pass over the tokens finds them all.
fn decoded_addresses(projection: &Projection, out: &mut Vec<Span>) -> Result<(), SpanError> {
    let text = projection.text();
    let mut found = Vec::new();
    for run in runs(text) {
        if let Some(word) = text.get(run.clone()) {
            found.extend(
                addresses(word)
                    .into_iter()
                    .map(|(a, b)| (run.start + a, run.start + b)),
            );
        }
    }
    if found.is_empty() {
        return Ok(());
    }
    found.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in found {
        match merged.last_mut() {
            Some((_, end)) if a <= *end => *end = (*end).max(b),
            _ => merged.push((a, b)),
        }
    }
    let mut next = 0;
    let mut at = 0;
    for token in projection.tokens() {
        let (from, to) = (at, at + token.text.len());
        at = to;
        while merged.get(next).is_some_and(|&(_, b)| b <= from) {
            next += 1;
        }
        let mut i = next;
        while let Some(&(a, b)) = merged.get(i)
            && a < to
        {
            let src = token.src;
            if !src.is_empty() {
                let exact = token.kind == TokenKind::Identity
                    && src.end() - src.start() == token.text.len();
                out.push(if exact {
                    Span::new(
                        src.start() + a.max(from) - from,
                        src.start() + b.min(to) - from,
                    )?
                } else {
                    src
                });
            }
            if b > to {
                break;
            }
            i += 1;
        }
    }
    Ok(())
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
    while i < bytes.len() {
        let opens = bytes[i] == b'<'
            && bytes
                .get(i + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'/' || *b == b'!');
        if !opens {
            i += 1;
            continue;
        }
        let mut quote = None;
        let mut j = i + 1;
        while j < bytes.len() {
            match (quote, bytes[j]) {
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
///
/// Each kind takes one pass over the run, so the scan is linear in its length.
fn addresses(run: &str) -> Vec<(usize, usize)> {
    const STOP: &[u8] = b"[]()<>\"',;";
    let scheme_char = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-');
    let bytes = run.as_bytes();
    let len = bytes.len();
    let mut found = Vec::new();
    // A URL starts after the last character before `://` that cannot be in a scheme.
    let mut scheme = 0;
    for (at, c) in run.char_indices() {
        if bytes.get(at..at + 3) == Some(b"://".as_slice()) {
            found.push((scheme, len));
        }
        if !(c.is_ascii_alphanumeric() || "+.-".contains(c)) {
            scheme = at + 1;
        }
    }
    // A `www.` host starts a word.
    let mut before: Option<char> = None;
    for (at, c) in run.char_indices() {
        if bytes.get(at..at + 4) == Some(b"www.".as_slice())
            && before.is_none_or(|c| !c.is_alphanumeric())
        {
            found.push((at, len));
        }
        before = Some(c);
    }
    // An email address runs from the stop character before its `@` to the one after it.
    let mut start = 0;
    let mut open = Vec::new();
    for (at, &b) in bytes.iter().enumerate() {
        if STOP.contains(&b) {
            found.extend(open.drain(..).map(|from| (from, at)));
            start = at + 1;
        } else if b == b'@' {
            open.push(start);
        }
    }
    found.extend(open.into_iter().map(|from| (from, len)));
    // An autolink is `<`, a scheme of two or more characters that starts with a letter, the
    // first `:` after the `<`, and a `>` later on. The pass runs backward, so the first `:`,
    // the first `>` and the first character that cannot be in a scheme after each byte are
    // known when the pass reaches it.
    let mut links = Vec::new();
    let (mut colon, mut close, mut other) = (None, None, None);
    for (at, &b) in bytes.iter().enumerate().rev() {
        if b == b'<'
            && let (Some(colon), Some(close)) = (colon, close)
            && colon > at + 2
            && other == Some(colon)
            && bytes.get(at + 1).is_some_and(u8::is_ascii_alphabetic)
        {
            links.push((at, close + 1));
        }
        if b == b':' {
            colon = Some(at);
        }
        if b == b'>' {
            close = Some(at);
        }
        if !scheme_char(b) {
            other = Some(at);
        }
    }
    found.extend(links.into_iter().rev());
    found
}
