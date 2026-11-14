//! HTML text extraction with exact byte positions.
//!
//! html5ever's tree builder, which follows the WHATWG tree construction rules, builds the
//! tree. It gets its tokens from html5gum, which reports source positions, and the tree
//! it builds carries the source range of every text run (see `scan` and `dom`). This module
//! walks that tree and turns it into blocks of prose with projections.
//!
//! A tree builder places text where a browser does, which is not always source order:
//! text in a table moves in front of it. A block never holds text out of source order. It
//! ends where the order breaks.
//!
//! Time and memory are linear in the input, with nesting capped at 512 as browsers do. The
//! walk keeps its own stack, so nesting never recurses.

mod dom;
mod scan;

use std::rc::Rc;

use html5ever::ns;
use slopvac_model::{SourceIndex, Span};

use crate::{Block, BlockKind, Document, ParseError, Projection, TokenKind};
pub use dom::Stats;
use dom::{Dom, Kind, PieceKind, Seg, TagKind};

const BOM: &str = "\u{feff}";

/// Placeholder text of a masked token. Rules must not read what it stands for.
const MASK: &str = "\0";

/// HTML elements that end the line of text around them.
const BREAKS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "br",
    "caption",
    "center",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "html",
    "legend",
    "li",
    "listing",
    "main",
    "menu",
    "nav",
    "ol",
    "optgroup",
    "option",
    "p",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "xmp",
];

/// Most tags to look at after an element's content for its end tag.
const END_SCAN: usize = 64;

/// HTML elements whose content is not prose.
const HIDDEN: &[&str] = &["script", "style", "noscript", "template", "head", "title"];

/// Parse the `body` range of HTML `source`.
///
/// `body` is where the caller found the content to read, after any front matter. Every
/// position in the result is a byte offset into `source`. Paragraphs, headings, list
/// items and table cells become blocks with a [`Projection`]. A `<pre>` element becomes a
/// code block. Inline `<code>`, `<kbd>` and `<samp>` are masked in the prose and listed in
/// [`Document::code_spans`]. `<script>`, `<style>`, `<noscript>`, `<template>`, `<head>`
/// and `<title>` contribute nothing.
///
/// # Errors
/// Returns [`ParseError::Body`] when `body` is not a valid span of `source`.
pub fn parse_html(source: &str, body: Span) -> Result<Document, ParseError> {
    parse_html_stats(source, body).map(|(document, _)| document)
}

/// Work the parse may do for each byte of input, in the units of [`Stats::work`].
///
/// Prose costs about 2 units a byte, and the worst tag soup in the test suite about 3. The
/// budget is 32, so ordinary and odd documents use a small part of it. Input that makes the
/// tree builder do far more per byte, such as thousands of tags that each close and reopen a
/// stack at the nesting cap, passes it and is refused. The test suite holds the corpus and
/// the generated documents under a quarter of the budget.
#[doc(hidden)]
pub const WORK_PER_BYTE: usize = 32;

/// The longest HTML body the parser accepts, in bytes.
#[doc(hidden)]
pub const MAX_HTML_BYTES: usize = 16 * 1024 * 1024;

/// Work the parse may do on top of that, for small documents.
#[doc(hidden)]
pub const WORK_BASE: usize = 4_096;

/// [`parse_html`], and counters for the work the tree builder did.
///
/// # Errors
/// Returns [`ParseError::Body`] when `body` is not a valid span of `source`.
#[doc(hidden)]
pub fn parse_html_stats(source: &str, body: Span) -> Result<(Document, Stats), ParseError> {
    parse_html_budget(source, body, WORK_PER_BYTE, WORK_BASE)
}

/// [`parse_html_stats`] with a work budget of `per_byte` units for each byte of the body
/// and `extra` on top.
///
/// # Errors
/// Returns [`ParseError::Body`] when `body` is not a valid span of `source`, and
/// [`ParseError::Budget`] when the document needs more work than the budget allows.
#[doc(hidden)]
pub fn parse_html_budget(
    source: &str,
    body: Span,
    per_byte: usize,
    extra: usize,
) -> Result<(Document, Stats), ParseError> {
    SourceIndex::disk(source).check_span(body)?;
    let base = if body.start() == 0 && source.starts_with(BOM) {
        BOM.len().min(body.end())
    } else {
        body.start()
    };
    let text = &source[base..body.end()];
    if text.len() > MAX_HTML_BYTES {
        return Err(ParseError::TooLarge);
    }
    let dom = scan::build(
        text,
        per_byte.saturating_mul(text.len()).saturating_add(extra),
    );
    let document = Walker::new(base, text).walk(&dom);
    if dom.exhausted.get() {
        return Err(ParseError::Budget);
    }
    let mut stats = dom.stats.get();
    stats.nodes = dom.nodes.borrow().len();
    stats.work = dom.work_done();
    Ok((document, stats))
}

/// Row and column counters of one `<table>`.
#[derive(Default)]
struct Table {
    row: Option<usize>,
    column: usize,
    /// The cell being read, as row and column. Each table keeps its own, so the cell of an
    /// outer table comes back when an inner table ends.
    cell: Option<(usize, usize)>,
}

enum Step {
    Enter(usize),
    Exit(usize),
}

/// Walks the tree in document order.
struct Walker<'a> {
    base: usize,
    text: &'a str,
    blocks: Vec<Block>,
    code_spans: Vec<Span>,
    prose: Prose,
    headings: Vec<u8>,
    tables: Vec<Table>,
}

impl<'a> Walker<'a> {
    fn new(base: usize, text: &'a str) -> Self {
        Self {
            base,
            text,
            blocks: Vec::new(),
            code_spans: Vec::new(),
            prose: Prose::new(base),
            headings: Vec::new(),
            tables: Vec::new(),
        }
    }

    /// Span of a range of the body text.
    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.base + start, self.base + end.max(start)).expect("start <= end")
    }

    fn walk(mut self, dom: &Rc<Dom>) -> Document {
        let mut stack = vec![Step::Enter(dom.document)];
        while let Some(step) = stack.pop() {
            dom.charge(1);
            if dom.exhausted.get() {
                break;
            }
            match step {
                Step::Enter(id) => self.enter(dom, id, &mut stack),
                Step::Exit(id) => self.exit(dom, id),
            }
        }
        self.flush();
        let Self {
            mut blocks,
            mut code_spans,
            ..
        } = self;
        blocks.sort_by_key(|b| (b.span.start(), std::cmp::Reverse(b.span.end())));
        code_spans.sort_by_key(Span::start);
        Document { blocks, code_spans }
    }

    /// Push the children of `id` so the first comes off the stack first.
    fn push_children(dom: &Dom, id: usize, stack: &mut Vec<Step>) {
        let nodes = dom.nodes.borrow();
        let mut at = nodes[id].last;
        while let Some(child) = at {
            stack.push(Step::Enter(child));
            at = nodes[child].prev;
        }
    }

    fn enter(&mut self, dom: &Dom, id: usize, stack: &mut Vec<Step>) {
        let element = {
            let nodes = dom.nodes.borrow();
            match &nodes[id].kind {
                Kind::Root => None,
                Kind::Text(segs) => {
                    let segs = segs.clone();
                    drop(nodes);
                    for seg in &segs {
                        self.seg(seg);
                    }
                    return;
                }
                Kind::Other => return,
                Kind::Element(e) => Some((Rc::clone(&e.name), e.start.clone())),
            }
        };
        let Some((name, start)) = element else {
            Self::push_children(dom, id, stack);
            return;
        };
        let html = name.ns == ns!(html);
        let local: &str = &name.local;
        if HIDDEN.contains(&local) && (html || matches!(local, "script" | "style" | "title")) {
            return;
        }
        if html && local == "pre" {
            self.flush();
            let (end, body_end) = self.extent(dom, id, &start);
            let kind = BlockKind::CodeBlock {
                lang: None,
                info: None,
                body: self.span(start.end.min(body_end), body_end),
                fenced: false,
            };
            self.blocks.push(Block {
                kind,
                span: self.span(start.start, end),
                projection: None,
            });
            return;
        }
        if html && matches!(local, "code" | "kbd" | "samp") {
            let (end, _) = self.extent(dom, id, &start);
            self.code_spans.push(self.span(start.start, end));
            self.reorder_check(start.start);
            self.prose
                .word(TokenKind::Masked, MASK, start.start, end.max(start.start));
            return;
        }
        if html {
            self.open(local);
        }
        stack.push(Step::Exit(id));
        Self::push_children(dom, id, stack);
    }

    fn exit(&mut self, dom: &Dom, id: usize) {
        let name = {
            let nodes = dom.nodes.borrow();
            let Kind::Element(e) = &nodes[id].kind else {
                return;
            };
            Rc::clone(&e.name)
        };
        if name.ns != ns!(html) {
            return;
        }
        let local: &str = &name.local;
        if !BREAKS.contains(&local) || local == "br" {
            return;
        }
        self.flush();
        match local {
            "table" => {
                self.tables.pop();
            }
            "td" | "th" => {
                if let Some(table) = self.tables.last_mut() {
                    table.cell = None;
                }
            }
            _ => {
                if heading_level(local).is_some() {
                    self.headings.pop();
                }
            }
        }
    }

    /// Start of an HTML element.
    fn open(&mut self, local: &str) {
        if local == "br" {
            self.prose.soft_break();
            return;
        }
        if !BREAKS.contains(&local) {
            return;
        }
        self.flush();
        match local {
            "table" => self.tables.push(Table::default()),
            "tr" => {
                if let Some(table) = self.tables.last_mut() {
                    table.row = Some(table.row.map_or(0, |r| r + 1));
                    table.column = 0;
                    table.cell = None;
                }
            }
            "td" | "th" => {
                if let Some(table) = self.tables.last_mut() {
                    let row = table.row.unwrap_or(0);
                    table.cell = Some((row, table.column));
                    table.column += 1;
                }
            }
            _ => {
                if let Some(level) = heading_level(local) {
                    self.headings.push(level);
                }
            }
        }
    }

    /// Where an element that the walk skips ends: the end of its end tag, and the start of
    /// that tag. The tree does not say, so this finds the end tag that follows the
    /// element's content. Only end tags and white space may sit between them. When there
    /// is none, the element ended by implication at the end of its content.
    fn extent(&self, dom: &Dom, id: usize, start: &std::ops::Range<usize>) -> (usize, usize) {
        let mut content = start.end;
        {
            let nodes = dom.nodes.borrow();
            let mut stack = vec![id];
            while let Some(at) = stack.pop() {
                let mut child = nodes[at].first;
                while let Some(c) = child {
                    dom.charge(1);
                    match &nodes[c].kind {
                        Kind::Text(segs) => {
                            for seg in segs {
                                content = content.max(seg.src.end);
                            }
                        }
                        Kind::Element(e) => {
                            content = content.max(e.start.end);
                            stack.push(c);
                        }
                        _ => {}
                    }
                    child = nodes[c].next;
                }
            }
        }
        let Kind::Element(e) = &dom.nodes.borrow()[id].kind else {
            return (content, content);
        };
        let local: &str = &e.name.local;
        let tags = dom.tags.borrow();
        let first = tags.partition_point(|t| t.range.start < content);
        let mut at = content;
        // Nothing between the content and its end tag but end tags, comments and white
        // space, so a few tags are enough. Stopping bounds the work for every element.
        for tag in tags[first..].iter().take(END_SCAN) {
            dom.charge(1);
            let mut stats = dom.stats.get();
            stats.end_scan += 1;
            dom.stats.set(stats);
            let gap = &self.text.as_bytes()[at.min(tag.range.start)..tag.range.start];
            if !gap.iter().all(|b| is_space(char::from(*b))) {
                break;
            }
            match &tag.kind {
                TagKind::End(name) if name == local => return (tag.range.end, tag.range.start),
                TagKind::End(_) | TagKind::Comment => at = tag.range.end,
                TagKind::Start => break,
            }
        }
        (content, content)
    }

    /// Text moved by foster parenting or reparenting can come out of source order. A block
    /// ends where it does.
    fn reorder_check(&mut self, start: usize) {
        if self.prose.last_end() > start {
            self.flush();
        }
    }

    fn seg(&mut self, seg: &Seg) {
        if seg.src.is_empty() {
            return;
        }
        let (start, end) = (seg.src.start, seg.src.end);
        self.reorder_check(start);
        if seg.kind == PieceKind::Plain {
            let slice = &self.text[start..end];
            let mut run: Option<(usize, bool)> = None;
            for (i, c) in slice.char_indices() {
                let space = is_space(c);
                match run {
                    Some((_, was)) if was == space => {}
                    Some((from, was)) => {
                        self.run(&slice[from..i], start + from, start + i, was);
                        run = Some((i, space));
                    }
                    None => run = Some((i, space)),
                }
            }
            if let Some((from, was)) = run {
                self.run(&slice[from..], start + from, end, was);
            }
        } else if seg.text.chars().all(is_space) {
            self.prose.space(start, end);
        } else {
            let kind = if seg.kind == PieceKind::Entity {
                TokenKind::Entity
            } else {
                TokenKind::Opaque
            };
            self.prose.word(kind, &seg.text, start, end);
        }
    }

    fn run(&mut self, text: &str, start: usize, end: usize, space: bool) {
        if space {
            self.prose.space(start, end);
        } else {
            self.prose.word(TokenKind::Identity, text, start, end);
        }
    }

    /// End the open block of prose.
    fn flush(&mut self) {
        let cell = self.tables.last().and_then(|t| t.cell);
        let kind = match (self.headings.last(), cell) {
            (Some(&level), _) => BlockKind::Heading { level },
            (None, Some((row, column))) => BlockKind::TableCell { row, column },
            (None, None) => BlockKind::Paragraph,
        };
        if let Some((projection, first, last)) = self.prose.take() {
            self.blocks.push(Block {
                kind,
                span: self.span(first, last),
                projection: Some(projection),
            });
        }
    }
}

fn heading_level(name: &str) -> Option<u8> {
    match name.as_bytes() {
        [b'h', d @ b'1'..=b'6'] => Some(d - b'0'),
        _ => None,
    }
}

/// Whitespace that HTML collapses.
const fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// A block of prose under construction.
struct Prose {
    base: usize,
    projection: Projection,
    /// An identity run not yet pushed, so adjacent text becomes one token.
    run: Option<(String, usize, usize)>,
    /// Whitespace waiting for the next word.
    space: Option<(usize, usize)>,
    /// A `<br>` waiting for the next word.
    separator: bool,
    first: Option<usize>,
    last: usize,
    /// Whether any token carries text a rule may read.
    readable: bool,
}

impl Prose {
    fn new(base: usize) -> Self {
        Self {
            base,
            projection: Projection::new(),
            run: None,
            space: None,
            separator: false,
            first: None,
            last: 0,
            readable: false,
        }
    }

    fn word(&mut self, kind: TokenKind, text: &str, start: usize, end: usize) {
        if self.first.is_some() {
            if let Some((from, to)) = self.space.take() {
                self.flush_run();
                self.push(TokenKind::Collapse, " ", from, to);
            } else if self.separator {
                self.flush_run();
                self.push(TokenKind::Wrap, " ", self.last, self.last);
            }
        }
        self.separator = false;
        self.space = None;
        if self.first.is_none() {
            self.first = Some(start);
        }
        if kind == TokenKind::Identity {
            match &mut self.run {
                Some((run, _, to)) if *to == start => {
                    run.push_str(text);
                    *to = end;
                }
                _ => {
                    self.flush_run();
                    self.run = Some((text.to_owned(), start, end));
                }
            }
            self.last = end;
            self.readable = true;
            return;
        }
        self.flush_run();
        self.readable |= kind != TokenKind::Masked;
        self.push(kind, text, start, end);
    }

    fn space(&mut self, start: usize, end: usize) {
        if self.first.is_some() {
            let from = self.space.map_or(start, |(from, _)| from);
            self.space = Some((from, end));
        }
    }

    /// Where the last text pushed or waiting ends.
    fn last_end(&self) -> usize {
        self.space.map_or(self.last, |(_, to)| to.max(self.last))
    }

    fn soft_break(&mut self) {
        self.separator = self.first.is_some();
    }

    fn flush_run(&mut self) {
        if let Some((text, start, end)) = self.run.take() {
            self.push(TokenKind::Identity, &text, start, end);
        }
    }

    fn push(&mut self, kind: TokenKind, text: &str, start: usize, end: usize) {
        let src =
            Span::new(self.base + start, self.base + end).expect("start <= end by construction");
        self.projection.push(kind, text, src);
        self.last = end;
    }

    /// The finished projection with the source range it covers, or `None` when no
    /// readable text came in. Trailing whitespace is dropped.
    fn take(&mut self) -> Option<(Projection, usize, usize)> {
        self.flush_run();
        let this = std::mem::replace(self, Self::new(self.base));
        let first = this.first?;
        this.readable.then_some((this.projection, first, this.last))
    }
}
