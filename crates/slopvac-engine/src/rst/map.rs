//! Map the HTML docutils wrote back to positions in the RST source.
//!
//! docutils drops markup, decodes escapes and joins soft-wrapped lines, so the HTML text
//! is not the source text. The mapper walks the two together. The HTML blocks come in
//! document order, and so do their words, so a cursor moves forward through the RST source
//! and each word is looked for after it.
//!
//! # Precision
//!
//! Positions are exact to the byte wherever a word of the HTML text is found in the RST
//! source:
//!
//! - A run of characters that appear in the source as they are becomes an `Identity`
//!   token.
//! - `\x` in the source becomes a `Backslash` token that covers both bytes, and `\ `
//!   (an escaped space) is a gap.
//! - Inline markup characters (`*` and a backquote) inside a word, as in `foo\ *bar*`,
//!   are gaps. Markup around a word is a gap too.
//! - The whitespace run that follows a word becomes a `Collapse` token, newline and
//!   indentation included.
//! - An inline literal becomes a `Masked` token over the literal and its backquotes, and
//!   is listed in [`Document::code_spans`].
//!
//! Text that is not in the source, such as a number docutils generates, is a synthetic
//! `Wrap` space. A word whose position cannot be found never receives a guessed one, so
//! a rule never reports a location that is wrong. A block with no findable word is left out.
//! The known cases:
//!
//! - The generated table of contents (`.. contents::`) repeats the section titles, so the
//!   mapper skips it. The titles themselves are mapped where they are written.
//! - Grid and simple tables whose cells span several lines have their text in source order
//!   that differs from the cell order of the HTML. Their words may not be found.
//! - A block's span covers the findable text, not the underline of a title or the
//!   markup of a directive.

use slopvac_model::Span;
use slopvac_parse::{Block, BlockKind, Document, Projection, TokenKind, parse_html};

use super::{RstError, span};

/// How far ahead the first word of a block may be, in bytes. Directive headers, comments
/// and literal blocks sit between blocks.
const FAR: usize = 16 * 1024;

/// How far ahead a later word of the same block may be, in bytes. A link target or an
/// inline literal can sit between two words.
const NEAR: usize = 1024;

/// Work, in characters looked at, per byte of source, plus a base for small documents.
const WORK_PER_BYTE: usize = 64;
const WORK_BASE: usize = 8 * 1024 * 1024;

/// A projection token before it is pushed.
type Tok = (TokenKind, String, Span);

/// A piece of the projected text of one block.
enum Item {
    Word(String),
    Space,
    Mask(Span),
}

/// Parse `html` and map the result into `rst`. Offsets below `bom` are the byte order mark.
pub(super) fn project(rst: &str, bom: usize, html: &str) -> Result<Document, RstError> {
    // The HTML parser masks `<code>`. The HTML 4 writer of docutils writes `<tt>`.
    let html = html
        .replace("<tt>", "<code>")
        .replace("<tt ", "<code ")
        .replace("</tt>", "</code>");
    let parsed = parse_html(&html, span(0, html.len()))?;
    let generated = generated_ranges(&html);
    let mut mapper = Mapper {
        src: rst,
        html: &html,
        cursor: bom,
        work: 0,
        budget: WORK_PER_BYTE
            .saturating_mul(rst.len())
            .saturating_add(WORK_BASE),
        code_spans: Vec::new(),
    };
    let mut blocks: Vec<Block> = Vec::new();
    for block in &parsed.blocks {
        if generated
            .iter()
            .any(|&(a, b)| block.span.start() >= a && block.span.start() < b)
        {
            continue;
        }
        let found_block = match &block.kind {
            BlockKind::CodeBlock {
                lang, info, fenced, ..
            } => mapper.code_block(block.span)?.map(|body| Block {
                kind: BlockKind::CodeBlock {
                    lang: lang.clone(),
                    info: info.clone(),
                    body,
                    fenced: *fenced,
                },
                span: body,
                projection: None,
            }),
            BlockKind::Paragraph | BlockKind::Heading { .. } | BlockKind::TableCell { .. } => {
                match &block.projection {
                    Some(projection) => mapper.prose(projection)?.map(|(projection, at)| Block {
                        kind: block.kind.clone(),
                        span: at,
                        projection: Some(projection),
                    }),
                    None => None,
                }
            }
            _ => None,
        };
        if let Some(done) = found_block {
            blocks.push(done);
        }
    }
    blocks.sort_by_key(|b| (b.span.start(), std::cmp::Reverse(b.span.end())));
    let mut code_spans = mapper.code_spans;
    code_spans.sort_by_key(Span::start);
    Ok(Document { blocks, code_spans })
}

struct Mapper<'a> {
    src: &'a str,
    html: &'a str,
    cursor: usize,
    work: usize,
    budget: usize,
    code_spans: Vec<Span>,
}

impl Mapper<'_> {
    fn charge(&mut self, units: usize) -> Result<(), RstError> {
        self.work = self.work.saturating_add(units);
        if self.work > self.budget {
            return Err(RstError::MappingBudget);
        }
        Ok(())
    }

    /// The end of the search window that starts at the cursor.
    fn limit(&self, window: usize) -> usize {
        let mut limit = self.cursor.saturating_add(window).min(self.src.len());
        while !self.src.is_char_boundary(limit) {
            limit += 1;
        }
        limit
    }

    /// Map the projection of one prose block.
    fn prose(&mut self, projection: &Projection) -> Result<Option<(Projection, Span)>, RstError> {
        let items = items(projection);
        let mut toks: Vec<Tok> = Vec::new();
        let mut located = false;
        let mut codes: Vec<Span> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let window = if located { NEAR } else { FAR };
            match item {
                Item::Space => {
                    if located && !ends_with_space(&toks) {
                        self.space(&mut toks);
                    }
                }
                Item::Word(word) => {
                    let glued_left = i > 0 && matches!(items[i - 1], Item::Mask(_));
                    let glued_right = matches!(items.get(i + 1), Some(Item::Mask(_)));
                    if let Some((found, end)) =
                        self.find_word(word, window, glued_left, glued_right)?
                    {
                        toks.extend(found);
                        self.cursor = end;
                        located = true;
                    } else if located && !ends_with_space(&toks) {
                        synthetic(&mut toks, self.cursor);
                    }
                }
                Item::Mask(at) => {
                    let literal = html_text(&self.html[at.start()..at.end()]);
                    if let Some(found) = self.find_literal(&literal, window)? {
                        toks.push((TokenKind::Masked, "\0".to_owned(), found));
                        codes.push(found);
                        self.cursor = found.end();
                        located = true;
                    } else if located && !ends_with_space(&toks) {
                        synthetic(&mut toks, self.cursor);
                    }
                }
            }
        }
        while ends_with_space(&toks) {
            toks.pop();
        }
        let (Some(lo), Some(hi)) = (
            toks.iter()
                .filter(|t| t.0.is_real())
                .map(|t| t.2.start())
                .min(),
            toks.iter()
                .filter(|t| t.0.is_real())
                .map(|t| t.2.end())
                .max(),
        ) else {
            return Ok(None);
        };
        let mut out = Projection::new();
        for (kind, text, at) in &toks {
            out.push(*kind, text, *at);
        }
        self.code_spans.extend(codes);
        Ok(Some((out, span(lo, hi))))
    }

    /// The whitespace after the cursor as a `Collapse` token, or a synthetic space.
    fn space(&mut self, toks: &mut Vec<Tok>) {
        let mut at = self.cursor;
        let rest = &self.src[at..];
        let closers = rest.len() - rest.trim_start_matches(['*', '`', '_']).len();
        let after = at + closers;
        let run = self.src[after..].len() - self.src[after..].trim_start().len();
        if run > 0 {
            at = after;
            toks.push((TokenKind::Collapse, " ".to_owned(), span(at, at + run)));
            self.cursor = at + run;
        } else {
            synthetic(toks, self.cursor);
        }
    }

    /// Find `word` after the cursor. Returns the tokens and the source end.
    fn find_word(
        &mut self,
        word: &str,
        window: usize,
        glued_left: bool,
        glued_right: bool,
    ) -> Result<Option<(Vec<Tok>, usize)>, RstError> {
        let Some(first) = word.chars().next() else {
            return Ok(None);
        };
        let limit = self.limit(window);
        let from = self.cursor;
        for (offset, ch) in self.src[from..limit].char_indices() {
            self.charge(1)?;
            if ch != first && ch != '\\' {
                continue;
            }
            let at = from + offset;
            if !glued_left
                && self.src[..at]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric)
            {
                continue;
            }
            if let Some((toks, end)) = match_word(self.src, at, word) {
                let boundary_ok = glued_right
                    || !word.chars().next_back().is_some_and(char::is_alphanumeric)
                    || !self.src[end..]
                        .chars()
                        .next()
                        .is_some_and(char::is_alphanumeric);
                if boundary_ok {
                    return Ok(Some((toks, end)));
                }
            }
        }
        Ok(None)
    }

    /// Find an inline literal after the cursor, with its backquotes.
    fn find_literal(&mut self, literal: &str, window: usize) -> Result<Option<Span>, RstError> {
        if literal.is_empty() {
            return Ok(None);
        }
        let from = self.cursor;
        let mut limit = self
            .limit(window)
            .saturating_add(literal.len())
            .min(self.src.len());
        while !self.src.is_char_boundary(limit) {
            limit += 1;
        }
        let Some(offset) = self.src[from..limit].find(literal) else {
            self.charge(limit - from)?;
            return Ok(None);
        };
        self.charge(offset + 1)?;
        let (start, end) = (from + offset, from + offset + literal.len());
        let before = self.src[..start]
            .bytes()
            .rev()
            .take(2)
            .take_while(|&b| b == b'`')
            .count();
        let after = self.src[end..]
            .bytes()
            .take(2)
            .take_while(|&b| b == b'`')
            .count();
        let ticks = before.min(after);
        Ok(Some(span(start - ticks, end + ticks)))
    }

    /// Map a `<pre>` block to the source span of its first and last findable lines.
    fn code_block(&mut self, html_span: Span) -> Result<Option<Span>, RstError> {
        let text = html_text(&self.html[html_span.start()..html_span.end()]);
        let mut first = None;
        let mut last = 0;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let window = if first.is_some() { NEAR } else { FAR };
            let Some(found) = self.find_literal_plain(line, window)? else {
                continue;
            };
            first.get_or_insert(found.start());
            last = found.end();
            self.cursor = found.end();
        }
        Ok(first.map(|start| span(start, last)))
    }

    /// Like [`Self::find_literal`], without the backquotes.
    fn find_literal_plain(&mut self, text: &str, window: usize) -> Result<Option<Span>, RstError> {
        let from = self.cursor;
        let mut limit = self
            .limit(window)
            .saturating_add(text.len())
            .min(self.src.len());
        while !self.src.is_char_boundary(limit) {
            limit += 1;
        }
        if let Some(offset) = self.src[from..limit].find(text) {
            self.charge(offset + 1)?;
            Ok(Some(span(from + offset, from + offset + text.len())))
        } else {
            self.charge(limit - from)?;
            Ok(None)
        }
    }
}

fn ends_with_space(toks: &[Tok]) -> bool {
    toks.last()
        .is_some_and(|t| t.1.ends_with(char::is_whitespace))
}

/// A synthetic space at the end of the last token, or at `fallback`.
fn synthetic(toks: &mut Vec<Tok>, fallback: usize) {
    let at = toks.last().map_or(fallback, |t| t.2.end());
    toks.push((TokenKind::Wrap, " ".to_owned(), Span::empty(at)));
}

/// Split the projected text into words, spaces and masked spans.
fn items(projection: &Projection) -> Vec<Item> {
    let mut items = Vec::new();
    let mut word = String::new();
    let flush = |items: &mut Vec<Item>, word: &mut String| {
        if !word.is_empty() {
            items.push(Item::Word(std::mem::take(word)));
        }
    };
    for token in projection.tokens() {
        if token.kind == TokenKind::Masked {
            flush(&mut items, &mut word);
            items.push(Item::Mask(token.src));
            continue;
        }
        for ch in token.text.chars() {
            if ch.is_whitespace() {
                flush(&mut items, &mut word);
                if !matches!(items.last(), Some(Item::Space)) {
                    items.push(Item::Space);
                }
            } else {
                word.push(ch);
            }
        }
    }
    flush(&mut items, &mut word);
    items
}

/// Match `word` at byte `at` of `src`, allowing escapes and inline markup characters.
fn match_word(src: &str, at: usize, word: &str) -> Option<(Vec<Tok>, usize)> {
    let mut toks: Vec<Tok> = Vec::new();
    let mut run = String::new();
    let mut run_start = at;
    let mut q = at;
    let flush = |toks: &mut Vec<Tok>, run: &mut String, run_start: usize, end: usize| {
        if !run.is_empty() {
            toks.push((
                TokenKind::Identity,
                std::mem::take(run),
                span(run_start, end),
            ));
        }
    };
    for (index, ch) in word.chars().enumerate() {
        loop {
            let c = src[q..].chars().next()?;
            if c == ch {
                if run.is_empty() {
                    run_start = q;
                }
                run.push(c);
                q += c.len_utf8();
                break;
            }
            match c {
                '\\' => {
                    let next = src[q + 1..].chars().next()?;
                    flush(&mut toks, &mut run, run_start, q);
                    if next == ch {
                        let end = q + 1 + next.len_utf8();
                        toks.push((TokenKind::Backslash, ch.to_string(), span(q, end)));
                        q = end;
                        break;
                    } else if next.is_whitespace() && index > 0 {
                        q += 1 + next.len_utf8();
                    } else {
                        return None;
                    }
                }
                '*' | '`' if index > 0 => {
                    flush(&mut toks, &mut run, run_start, q);
                    q += 1;
                }
                _ => return None,
            }
        }
    }
    flush(&mut toks, &mut run, run_start, q);
    Some((toks, q))
}

/// The text of an HTML fragment: tags removed and entities decoded.
fn html_text(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut rest = fragment;
    while let Some(i) = rest.find(['<', '&']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if rest.starts_with('<') {
            rest = rest.find('>').map_or("", |j| &rest[j + 1..]);
            continue;
        }
        let decoded = rest[1..]
            .find(';')
            .filter(|&j| j <= 10)
            .and_then(|j| entity(&rest[1..j + 2]).map(|c| (c, j + 2)));
        if let Some((c, used)) = decoded {
            out.push(c);
            rest = &rest[used..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// The character of an entity name such as `amp;`, or `None`.
fn entity(name_semicolon: &str) -> Option<char> {
    let name = name_semicolon.strip_suffix(';')?;
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let digits = name.strip_prefix('#')?;
            let value = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(value)
        }
    }
}

/// Byte ranges of the HTML that docutils generated from the source, so no source text
/// stands behind them. Today that is the table of contents.
fn generated_ranges(html: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(i) = html[from..].find("class=\"contents") {
        let at = from + i;
        let Some(open) = html[..at].rfind('<') else {
            break;
        };
        let name: String = html[open + 1..]
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        let end = element_end(html, open, &name);
        ranges.push((open, end));
        from = end.max(at + 1);
    }
    ranges
}

/// The end of the element that opens at `open`, counting nested elements of one name.
fn element_end(html: &str, open: usize, name: &str) -> usize {
    let opener = format!("<{name}");
    let closer = format!("</{name}>");
    let mut depth = 0_usize;
    let mut at = open;
    while let Some(i) = html[at..].find('<') {
        let tag = &html[at + i..];
        if tag.starts_with(&closer) {
            depth = depth.saturating_sub(1);
            at += i + closer.len();
            if depth == 0 {
                return at;
            }
        } else if tag.starts_with(&opener) && tag[opener.len()..].starts_with([' ', '>', '/', '\n'])
        {
            depth += 1;
            at += i + opener.len();
        } else {
            at += i + 1;
        }
    }
    html.len()
}
