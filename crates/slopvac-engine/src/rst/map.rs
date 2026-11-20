//! Place the prose of a docutils tree in the RST source.
//!
//! docutils records the source lines of every block, and keeps the source text of every
//! inline construct (`rawsource`). So the mapper never searches the whole file. Each node is
//! placed inside its own lines, one piece after the other:
//!
//! - Text is matched character by character. A `\x` escape becomes a `backslash` token, an
//!   escaped space is a gap, and a run of whitespace becomes a `collapse` token that covers
//!   the newline and the indentation.
//! - An inline construct (emphasis, a link, a role) must start where the text before it
//!   ended. Its words are matched inside its own source text, so the markup
//!   around them stays a gap.
//! - Inline literals become `masked` tokens over the literal with its backquotes.
//!
//! The first piece of a node may follow a list marker, a directive header or a table
//! border, so the mapper looks for its first word inside the node's lines only.
//!
//! When a node cannot be placed word by word, its prose is kept and maps to the whole
//! trimmed lines of the node (an `opaque` token): the position is line-precise. A first
//! attempt allows one line of slack on each side, because docutils 0.21 puts the line of
//! some directive output (`rubric`) one line late. A node with no usable lines is reported
//! in a notice. No prose is dropped.
//!
//! # Accounting
//!
//! Source text and driver output are read only through the metered types in `meter`, which
//! count every byte they examine. The budget grows with the size of the source and the text.
//! Past it, nodes fall back to line precision without any more matching, so the cost stays
//! linear. The `scanned` total keeps counting after that, and the property tests bound it.

use std::collections::HashMap;

use slopvac_model::Span;
use slopvac_parse::{
    Admonition, AdmonitionSyntax, Block, BlockKind, Container as ParseContainer, Document,
    Projection, TokenKind,
};

use super::meter::{Cur, Meter, Src};
use super::model::{Container, Node, Piece};
use super::{RstNotice, span};

/// Characters looked at per byte of source and text, plus a base for small documents.
pub(super) const WORK_PER_BYTE: usize = 64;
/// See [`WORK_PER_BYTE`].
pub(super) const WORK_BASE: usize = 1 << 20;

/// A projection token before it is pushed.
type Tok = (TokenKind, String, Span);

/// The result of mapping one document.
pub(super) struct Mapped {
    pub(super) document: Document,
    pub(super) notices: Vec<RstNotice>,
    /// Characters charged to the budget.
    pub(super) work: usize,
    /// Bytes examined by every read of the source and of the driver output, charged or not.
    /// The budget caps the charged part; the rest is a small multiple of the input.
    pub(super) scanned: usize,
    /// Source spans a fix must not touch: everything but the words of prose that were
    /// placed exactly, and the markup, link targets and code inside those words.
    pub(super) protected: Vec<Span>,
}

/// Map `nodes` into `source`. `bom` is the byte length of a leading byte order mark.
pub(super) fn project(source: &str, bom: usize, nodes: &[Node]) -> Mapped {
    let text_bytes: usize = nodes
        .iter()
        .map(|n| {
            n.text.len()
                + n.pieces
                    .iter()
                    .map(|p| p.t.len() + p.raw.len())
                    .sum::<usize>()
        })
        .sum();
    let mut meter = Meter::new(
        WORK_PER_BYTE
            .saturating_mul(source.len().saturating_add(text_bytes))
            .saturating_add(WORK_BASE),
    );
    // The driver output was decoded: its bytes are examined once here.
    meter.scan(text_bytes);
    let src = Src::new(source);
    let starts = src.line_starts(&mut meter, bom);
    let mut mapper = Mapper::new(src, starts, meter);
    let mut blocks = Vec::new();
    let mut code_spans = Vec::new();
    let mut unplaced: Vec<&str> = Vec::new();
    let mut line_precise = 0_usize;
    // Where a fix may write, and spans inside it that it may not touch.
    let mut allowed: Vec<Span> = Vec::new();
    let mut locked: Vec<Span> = Vec::new();
    for node in nodes {
        if node.mark {
            locked.extend(mapper.mark(node));
            continue;
        }
        let tag = node.tag.as_str();
        match mapper.block(node) {
            Outcome::Unplaced => unplaced.push(&node.tag),
            Outcome::Empty => {}
            Outcome::Block {
                block,
                codes,
                locks,
                precise,
            } => {
                line_precise += usize::from(precise);
                if !precise && is_prose(&block, tag) {
                    allowed.push(block.span);
                }
                locked.extend(locks);
                locked.extend(codes.iter().copied());
                code_spans.extend(codes);
                blocks.push(block);
            }
        }
    }
    blocks.sort_by_key(|b| (b.span.start(), std::cmp::Reverse(b.span.end())));
    code_spans.sort_by_key(Span::start);
    let mut notices = Vec::new();
    if !unplaced.is_empty() {
        let message = crate::messages::EngineNotice::RstUnplaced {
            message: crate::messages::EngineDetails::RstUnplaced {
                count: unplaced.len(),
                first: unplaced[0].into(),
            }
            .to_string(),
        };
        notices.push(RstNotice {
            code: slopvac_messages::Catalog::code(&message),
            warning: true,
            message: message.to_string(),
        });
    }
    if line_precise > 0 {
        let message = crate::messages::EngineNotice::RstLinePrecise {
            message: crate::messages::EngineDetails::RstLinePrecise {
                count: line_precise,
            }
            .to_string(),
        };
        notices.push(RstNotice {
            code: slopvac_messages::Catalog::code(&message),
            warning: false,
            message: message.to_string(),
        });
    }
    Mapped {
        document: Document {
            blocks,
            code_spans,
            html_blocks: Vec::new(),
        },
        notices,
        work: mapper.m.work(),
        scanned: mapper.m.scanned(),
        protected: protected(allowed, locked, source.len()),
    }
}

/// Whether a fix may write in a block. Only the nodes docutils builds from running prose are
/// listed; definitions, targets and every tag not named here are never writable, so a new
/// node type is protected until someone decides otherwise.
fn is_prose(block: &Block, tag: &str) -> bool {
    block.projection.is_some()
        && matches!(
            tag,
            "paragraph" | "title" | "subtitle" | "term" | "classifier" | "line" | "attribution"
        )
}

/// The spans a fix must not touch: `locked`, and every part of the `len`-byte source that
/// is outside `allowed`. Directive markup, options, literal blocks, comments, targets,
/// substitution definitions and every block that could not be placed exactly are in the
/// second part, because only exactly placed prose is allowed.
fn protected(mut allowed: Vec<Span>, mut locked: Vec<Span>, len: usize) -> Vec<Span> {
    allowed.sort_by_key(Span::start);
    let mut reached = 0;
    for a in allowed {
        if a.start() > reached {
            locked.push(span(reached, a.start()));
        }
        reached = reached.max(a.end());
    }
    if reached < len {
        locked.push(span(reached, len));
    }
    locked.sort_by_key(Span::start);
    locked
}

/// The columns of a table border line.
enum Border {
    /// Character columns of the `+` signs of a grid table.
    Grid(Vec<usize>),
    /// Character columns where each `=` run of a simple table starts.
    Simple(Vec<usize>),
}

impl Border {
    /// The character columns of the cell in `column`, widened over `columns` columns.
    fn window(&self, column: usize, columns: usize) -> Option<(usize, usize)> {
        let end = column + columns.max(1);
        match self {
            Self::Grid(pluses) => Some((*pluses.get(column)? + 1, *pluses.get(end)?)),
            Self::Simple(starts) => Some((
                *starts.get(column)?,
                starts.get(end).copied().unwrap_or(usize::MAX),
            )),
        }
    }
}

/// Read a grid (`+----+----+`) or simple (`====  ====`) table border. The caller has paid
/// for the bytes of `line`.
fn parse_border(line: &str) -> Option<Border> {
    let text = line.trim_end();
    let body = text.trim_start();
    if body.is_empty() {
        return None;
    }
    if body.starts_with('+') && body.ends_with('+') && body.chars().all(|c| "+-=:".contains(c)) {
        let pluses = text
            .chars()
            .enumerate()
            .filter(|&(_, c)| c == '+')
            .map(|(i, _)| i)
            .collect();
        return Some(Border::Grid(pluses));
    }
    if body.starts_with('=') && body.chars().all(|c| c == '=' || c == ' ') {
        let mut starts = Vec::new();
        let mut previous = ' ';
        for (i, c) in text.chars().enumerate() {
            if c == '=' && previous == ' ' {
                starts.push(i);
            }
            previous = c;
        }
        return Some(Border::Simple(starts));
    }
    None
}

/// What became of a node. One value is alive at a time, so the size of the variants does not
/// matter.
#[allow(clippy::large_enum_variant)]
enum Outcome {
    /// Its lines are missing or outside the file.
    Unplaced,
    /// It holds no prose.
    Empty,
    /// A block, with the spans of its inline code and whether it is line-precise.
    Block {
        block: Block,
        codes: Vec<Span>,
        /// Spans inside the block that a fix must not touch.
        locks: Vec<Span>,
        precise: bool,
    },
}

/// A node placed in the source.
struct Placed {
    projection: Projection,
    span: Span,
    codes: Vec<Span>,
    locks: Vec<Span>,
    line_precise: bool,
}

/// Tokens of one attempt.
struct Attempt {
    toks: Vec<Tok>,
    codes: Vec<Span>,
    locks: Vec<Span>,
}

struct Mapper<'a> {
    src: Src<'a>,
    starts: Vec<usize>,
    m: Meter,
    /// For a line, the nearest table border at or above it (`None`: there is none).
    memo: HashMap<usize, Option<usize>>,
    /// For a line, where its text starts and ends without the whitespace around it.
    trims: HashMap<usize, Option<(usize, usize)>>,
    /// The table borders read so far, by line.
    borders: HashMap<usize, Border>,
    /// For a line that starts with a field name, where the name ends.
    names: HashMap<usize, usize>,
}

impl<'a> Mapper<'a> {
    fn new(src: Src<'a>, starts: Vec<usize>, m: Meter) -> Self {
        Self {
            src,
            starts,
            m,
            memo: HashMap::new(),
            trims: HashMap::new(),
            borders: HashMap::new(),
            names: HashMap::new(),
        }
    }

    /// The block for `node`.
    fn block(&mut self, node: &Node) -> Outcome {
        let Some((lo, hi)) = node
            .start
            .zip(node.end)
            .and_then(|(s, e)| self.byte_range(s, e))
        else {
            return Outcome::Unplaced;
        };
        let containers: Vec<ParseContainer> = node
            .containers
            .iter()
            .map(|c| match *c {
                Container::Item { ordered, depth } => ParseContainer::ListItem { ordered, depth },
                Container::Quote => ParseContainer::BlockQuote,
            })
            .collect();
        let structural = |kind| Outcome::Block {
            block: Block {
                kind,
                span: span(lo, hi),
                projection: None,
                containers: containers.clone(),
            },
            codes: Vec::new(),
            locks: Vec::new(),
            precise: false,
        };
        if node.table {
            // The border lines above and below belong to the table.
            let (first, last) = (node.start.unwrap_or(1), node.end.unwrap_or(1));
            let top = first > 1 && self.is_border(first - 1);
            let bottom = last < self.starts.len() && self.is_border(last + 1);
            let (lo, hi) = self
                .byte_range(first - usize::from(top), last + usize::from(bottom))
                .unwrap_or((lo, hi));
            return Outcome::Block {
                block: Block {
                    kind: BlockKind::Table,
                    span: span(lo, hi),
                    projection: None,
                    containers,
                },
                codes: Vec::new(),
                locks: Vec::new(),
                precise: false,
            };
        }
        if node.code {
            return structural(BlockKind::CodeBlock {
                lang: node.lang.clone(),
                info: None,
                body: span(lo, hi),
                fenced: false,
            });
        }
        if let Some(kind) = &node.adm {
            return structural(BlockKind::Admonition(Admonition {
                kind: kind.clone(),
                title: node.title.clone(),
                title_span: None,
                syntax: AdmonitionSyntax::Directive,
            }));
        }
        let after_name = node
            .start
            .and_then(|s| self.names.get(&s).copied())
            .filter(|&end| lo < end && end <= hi);
        let Some(mut placed) = self.place(node, after_name.unwrap_or(lo), hi) else {
            return Outcome::Empty;
        };
        self.lock_borders(&mut placed);
        let kind = match (node.level, node.cell) {
            (Some(level), _) => BlockKind::Heading { level },
            (None, Some((row, column, _))) => BlockKind::TableCell { row, column },
            _ => BlockKind::Paragraph,
        };
        Outcome::Block {
            block: Block {
                kind,
                span: placed.span,
                projection: Some(placed.projection),
                containers,
            },
            codes: placed.codes,
            locks: placed.locks,
            precise: placed.line_precise,
        }
    }

    /// Lock what a token over a gap holds besides whitespace: the line breaks and the table
    /// borders. An edit that crosses a wrapped line of a table cell crosses the words of the
    /// cells beside it, so it must touch a line break; the words themselves stay fixable
    /// through their own blocks.
    fn lock_borders(&mut self, placed: &mut Placed) {
        for t in placed.projection.tokens() {
            if !matches!(t.kind, TokenKind::Collapse) || t.src.is_empty() {
                continue;
            }
            let at = t.src.start();
            let text = self.src.slice_free(&mut self.m, at, t.src.end());
            if text.chars().all(char::is_whitespace) {
                continue;
            }
            for (i, c) in text.char_indices() {
                if matches!(c, '\n' | '\r' | '|' | '+') {
                    placed.locks.push(span(at + i, at + i + 1));
                }
            }
        }
    }

    /// The part of a field's first line that is its name, `:name:`. When the name cannot
    /// be found there, the whole line is locked, which also locks the start of the body.
    fn mark(&mut self, node: &Node) -> Option<Span> {
        let line = node.start?;
        if line == 0 || line > self.starts.len() {
            return None;
        }
        let (from, to) = self.line_content(line)?;
        if !self.m.exhausted() {
            let text = self.line_text(line);
            let lead = text.len() - text.trim_start().len();
            let rest = &text[lead..];
            let named = rest
                .strip_prefix(':')
                .and_then(|r| r.strip_prefix(node.text.as_str()))
                .is_some_and(|r| r.starts_with(':'));
            if named {
                let at = self.starts[line - 1] + lead;
                let end = at + 1 + node.text.len() + 1;
                self.names.insert(line, end);
                return Some(span(at, end));
            }
        }
        Some(span(from, to))
    }

    /// Byte range of lines `first..=last` (1-based), without the final terminator.
    fn byte_range(&self, first: usize, last: usize) -> Option<(usize, usize)> {
        let lines = self.starts.len();
        if first == 0 || first > lines || last < first {
            return None;
        }
        let last = last.min(lines);
        Some((self.starts[first - 1], self.line_end(last)))
    }

    /// End of line `line` (1-based) before its terminator.
    fn line_end(&self, line: usize) -> usize {
        if line >= self.starts.len() {
            return self.src.len();
        }
        self.src.line_end_before(self.starts[line])
    }

    /// The text of line `line` (1-based), without its terminator. Its bytes are charged.
    fn line_text(&mut self, line: usize) -> &'a str {
        let (a, b) = (self.starts[line - 1], self.line_end(line));
        self.src.slice(&mut self.m, a, b)
    }

    /// Place a prose node inside lines `lo..hi`.
    fn place(&mut self, node: &Node, lo: usize, hi: usize) -> Option<Placed> {
        if let Some((_, column, columns)) = node.cell
            && let Some(window) = self.column_window(node, column, columns)
        {
            return self.place_in_column(node, window);
        }
        if !self.m.exhausted() {
            let (first, last) = (node.start?, node.end?);
            let wider = self.byte_range(first.saturating_sub(1).max(1), last + 1);
            for range in [Some((lo, hi)), wider] {
                let Some((a, b)) = range else { continue };
                if let Some(done) = self.align(&node.pieces, a, b) {
                    return Self::finish(done);
                }
            }
        }
        self.fallback_lines(&node.pieces, node.start?, node.end?)
    }

    /// Whether `line` is a table border.
    fn is_border(&mut self, line: usize) -> bool {
        let text = self.line_text(line);
        !self.m.exhausted() && parse_border(text).is_some()
    }

    /// The character columns `start..end` of the cell's columns, read from the nearest
    /// border line above the node.
    fn column_window(
        &mut self,
        node: &Node,
        column: usize,
        columns: usize,
    ) -> Option<(usize, usize)> {
        let line = self.nearest_border(node.start?)?;
        self.borders.get(&line)?.window(column, columns)
    }

    /// The nearest table border above `first`, stopping at a blank line. Every line is
    /// read and charged once for the whole document: the answer for each line is kept, and
    /// the parsed borders too.
    fn nearest_border(&mut self, first: usize) -> Option<usize> {
        let mut trail = Vec::new();
        let mut line = first.min(self.starts.len() + 1);
        let found = loop {
            if line <= 1 {
                break None;
            }
            line -= 1;
            if let Some(&known) = self.memo.get(&line) {
                break known;
            }
            let text = self.line_text(line);
            if self.m.exhausted() {
                return None;
            }
            if text.trim().is_empty() {
                trail.push(line);
                break None;
            }
            if let Some(border) = parse_border(text) {
                self.borders.insert(line, border);
                trail.push(line);
                break Some(line);
            }
            trail.push(line);
        };
        for l in trail {
            self.memo.insert(l, found);
        }
        found
    }

    /// Place a table cell. Only the cell's columns of each of its lines are read: they
    /// are joined into a view, the words of neighbouring cells cannot match, and the
    /// result is mapped back line by line.
    fn place_in_column(&mut self, node: &Node, (x0, x1): (usize, usize)) -> Option<Placed> {
        let (first, last) = (node.start?, node.end?.min(self.starts.len()));
        let mut view = String::new();
        // (start in the view, start in the source, length) of each line's piece.
        let mut pieces: Vec<(usize, usize, usize)> = Vec::new();
        for line in first..=last {
            let (a, b) = (self.starts[line - 1], self.line_end(line));
            let Some((from, to)) = self.src.window(&mut self.m, a, b, x0, x1) else {
                return self.fallback_lines(&node.pieces, first, last);
            };
            let text = self.src.slice(&mut self.m, from, to);
            pieces.push((view.len(), from, to - from));
            view.push_str(text);
            view.push('\n');
        }
        let mut inner = Mapper::new(Src::new(&view), vec![0], self.m.child());
        let placed = if inner.m.exhausted() {
            None
        } else {
            inner
                .align(&node.pieces, 0, view.len())
                .and_then(Self::finish)
        }
        .or_else(|| inner.fallback(&node.pieces, 0, view.len()));
        self.m.absorb(&inner.m);
        let start_of = |v: usize| {
            let i = pieces
                .partition_point(|&(at, _, _)| at <= v)
                .saturating_sub(1);
            let (at, src, len) = pieces[i];
            src + (v - at).min(len)
        };
        let end_of = |e: usize| {
            if e == 0 || view.as_bytes()[e - 1] == b'\n' {
                start_of(e)
            } else {
                start_of(e - 1) + 1
            }
        };
        let shift = |s: Span| {
            if s.is_empty() {
                span(start_of(s.start()), start_of(s.start()))
            } else {
                span(start_of(s.start()), end_of(s.end()))
            }
        };
        placed.map(|p| {
            let mut projection = Projection::new();
            for t in p.projection.tokens() {
                // A token over a line break of the view covers the cells beside this one.
                let bridges = view.as_bytes()[t.src.start()..t.src.end()].contains(&b'\n');
                if bridges {
                    projection.push_bridge(t.kind, &t.text, shift(t.src));
                } else {
                    projection.push(t.kind, &t.text, shift(t.src));
                }
            }
            Placed {
                projection,
                span: shift(p.span),
                codes: p.codes.into_iter().map(shift).collect(),
                locks: p.locks.into_iter().map(shift).collect(),
                line_precise: p.line_precise,
            }
        })
    }

    fn finish(done: Attempt) -> Option<Placed> {
        let Attempt {
            mut toks,
            codes,
            locks,
        } = done;
        while toks
            .last()
            .is_some_and(|t| t.1.ends_with(char::is_whitespace))
        {
            toks.pop();
        }
        let lo = toks
            .iter()
            .filter(|t| t.0.is_real())
            .map(|t| t.2.start())
            .min()?;
        let hi = toks
            .iter()
            .filter(|t| t.0.is_real())
            .map(|t| t.2.end())
            .max()?;
        let mut projection = Projection::new();
        for (kind, text, at) in &toks {
            projection.push(*kind, text, *at);
        }
        Some(Placed {
            projection,
            span: span(lo, hi),
            codes,
            locks,
            line_precise: false,
        })
    }

    /// The prose of a node as plain text with its inline code as `\0`, whitespace collapsed.
    /// The pieces are driver output, counted once when it was decoded.
    fn fallback_text(pieces: &[Piece]) -> Option<String> {
        let mut text = String::new();
        for piece in pieces {
            match piece.k.as_str() {
                "t" | "i" => text.push_str(&piece.t),
                "c" => text.push('\0'),
                _ => {}
            }
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then_some(text)
    }

    fn fallback_at(text: &str, start: usize, end: usize) -> Placed {
        let at = span(start, end.max(start));
        let mut projection = Projection::new();
        projection.push(TokenKind::Opaque, text, at);
        Placed {
            projection,
            span: at,
            codes: Vec::new(),
            locks: Vec::new(),
            line_precise: true,
        }
    }

    /// The prose of the node as one token over `lo..hi`, trimmed. The range is the node's
    /// own text (a view), so trimming it is bounded by that text.
    fn fallback(&mut self, pieces: &[Piece], lo: usize, hi: usize) -> Option<Placed> {
        let text = Self::fallback_text(pieces)?;
        let (start, end) = self.src.trimmed(&mut self.m, lo, hi).unwrap_or((lo, lo));
        Some(Self::fallback_at(&text, start, end))
    }

    /// The prose of the node as one token over its lines `first..=last`, without the
    /// whitespace at the ends. Cells of one row share lines, so each line is trimmed once.
    fn fallback_lines(&mut self, pieces: &[Piece], first: usize, last: usize) -> Option<Placed> {
        let text = Self::fallback_text(pieces)?;
        let last = last.min(self.starts.len());
        let mut start = None;
        for line in first..=last {
            self.m.scan(1);
            if let Some((from, _)) = self.line_content(line) {
                start = Some(from);
                break;
            }
        }
        let mut end = None;
        for line in (first..=last).rev() {
            self.m.scan(1);
            if let Some((_, to)) = self.line_content(line) {
                end = Some(to);
                break;
            }
        }
        let (start, end) = start.zip(end).unwrap_or_else(|| {
            let at = self.starts[first.min(self.starts.len()).max(1) - 1];
            (at, at)
        });
        Some(Self::fallback_at(&text, start, end))
    }

    /// The part of line `line` between its first and last non-whitespace character, as source
    /// offsets, or `None` for a blank line. Each line is read once.
    fn line_content(&mut self, line: usize) -> Option<(usize, usize)> {
        if let Some(known) = self.trims.get(&line) {
            return *known;
        }
        let (a, b) = (self.starts[line - 1], self.line_end(line));
        self.src.slice_free(&mut self.m, a, b);
        let found = self.src.trimmed(&mut self.m, a, b);
        self.trims.insert(line, found);
        found
    }

    /// Match all pieces of a node inside `lo..hi`.
    fn align(&mut self, pieces: &[Piece], lo: usize, hi: usize) -> Option<Attempt> {
        let mut out = Attempt {
            toks: Vec::new(),
            codes: Vec::new(),
            locks: Vec::new(),
        };
        let mut cursor = lo;
        let mut first = true;
        for piece in pieces {
            match piece.k.as_str() {
                "t" => {
                    let text = if first {
                        piece.t.trim_start()
                    } else {
                        &piece.t
                    };
                    if text.is_empty() {
                        continue;
                    }
                    if first {
                        cursor = self.find_word_start(cursor, hi, text)?;
                    }
                    cursor = self.text(text, cursor, hi, &mut out.toks)?;
                    first = false;
                }
                "c" | "i" | "o" => {
                    if piece.raw.is_empty() {
                        // Nothing to anchor on: treat the text as plain text.
                        if piece.k == "o" || piece.t.is_empty() {
                            continue;
                        }
                        if first {
                            cursor = self.find_word_start(cursor, hi, &piece.t)?;
                        }
                        let begin = cursor;
                        cursor = self.text(&piece.t, cursor, hi, &mut out.toks)?;
                        if piece.lock {
                            out.locks.push(span(begin, cursor));
                        }
                        first = false;
                        continue;
                    }
                    let begin = if first {
                        self.find_raw(cursor, hi, &piece.raw)?
                    } else {
                        self.skip_gap(cursor, hi)?
                    };
                    let end = self.eat_raw(begin, hi, &piece.raw)?;
                    first = false;
                    match piece.k.as_str() {
                        "c" => {
                            let at = span(begin, end);
                            push(&mut out.toks, (TokenKind::Masked, "\0".to_owned(), at));
                            out.codes.push(at);
                        }
                        "i" => {
                            let words = self.inline(piece, begin, end, &mut out.toks);
                            if piece.lock {
                                // The words name the target or belong to interpreted text.
                                out.locks.push(span(begin, end));
                            } else if let Some((from, to)) = words {
                                // Only the markup around the words is locked.
                                out.locks.push(span(begin, from));
                                out.locks.push(span(to, end));
                            } else {
                                out.locks.push(span(begin, end));
                            }
                        }
                        _ => out.locks.push(span(begin, end)),
                    }
                    cursor = end;
                }
                _ => return None,
            }
        }
        Some(out)
    }

    /// Place the words of inline markup inside its own source text.
    ///
    /// Returns the source range of the words, or `None` when they stayed one opaque token
    /// over the whole construct.
    fn inline(
        &mut self,
        piece: &Piece,
        begin: usize,
        end: usize,
        toks: &mut Vec<Tok>,
    ) -> Option<(usize, usize)> {
        let raw = self.src.slice(&mut self.m, begin, end);
        let mut skip = 0;
        if raw.starts_with(':')
            && let Some(i) = raw[1..].find(':')
        {
            skip = i + 2;
        }
        skip += raw[skip..].len() - raw[skip..].trim_start_matches(['*', '`', '_']).len();
        let mut inner = Vec::new();
        if self.text(&piece.t, begin + skip, end, &mut inner).is_some() {
            let real = || inner.iter().filter(|t| t.0.is_real());
            let words = real()
                .map(|t| t.2.start())
                .min()
                .zip(real().map(|t| t.2.end()).max());
            for tok in inner.iter().cloned() {
                push(toks, tok);
            }
            words
        } else {
            push(toks, (TokenKind::Opaque, piece.t.clone(), span(begin, end)));
            None
        }
    }

    /// Skip `\ ` and `\` + newline pairs, which docutils removes. Every pair is charged.
    fn skip_gap(&mut self, mut at: usize, hi: usize) -> Option<usize> {
        while self.src.peek(&mut self.m, at, hi) == Some('\\') {
            let next = self.src.peek(&mut self.m, at + 1, hi)?;
            if !next.is_whitespace() {
                break;
            }
            self.m.spend(1)?;
            at += 1 + next.len_utf8();
        }
        Some(at)
    }

    /// The first position in `from..hi` where `raw` starts.
    fn find_raw(&mut self, from: usize, hi: usize, raw: &str) -> Option<usize> {
        let first = Cur::new(raw).peek(&mut self.m)?;
        let mut at = from;
        while at < hi {
            let (off, _) = self.src.find(&mut self.m, at, hi, |c| c == first)?;
            if self.m.exhausted() {
                return None;
            }
            at += off;
            if self.eat_raw(at, hi, raw).is_some() {
                return Some(at);
            }
            at += first.len_utf8();
        }
        None
    }

    /// Match `raw` at `at`. Whitespace in `raw` is one space, which matches any run of
    /// whitespace in the source, because docutils strips the indentation of continuation
    /// lines. Returns the end.
    fn eat_raw(&mut self, at: usize, hi: usize, raw: &str) -> Option<usize> {
        let mut p = at;
        let mut cur = Cur::new(raw);
        while let Some(ch) = cur.bump(&mut self.m) {
            if self.m.exhausted() {
                return None;
            }
            let c = self.src.peek(&mut self.m, p, hi)?;
            if ch.is_whitespace() {
                if !c.is_whitespace() {
                    return None;
                }
                cur.skip_ws(&mut self.m);
                p = self.src.skip_while(&mut self.m, p, hi, char::is_whitespace);
            } else if c == ch {
                p += c.len_utf8();
            } else {
                return None;
            }
        }
        Some(p)
    }

    /// The first position in `from..hi` where the first word of `text` starts as a word.
    fn find_word_start(&mut self, from: usize, hi: usize, text: &str) -> Option<usize> {
        let mut cur = Cur::new(text);
        let mut word = String::new();
        while let Some(c) = cur.peek(&mut self.m) {
            if c.is_whitespace() {
                break;
            }
            word.push(c);
            cur.bump(&mut self.m);
        }
        let first = word.chars().next()?;
        let mut at = from;
        while at < hi {
            let (off, ch) = self
                .src
                .find(&mut self.m, at, hi, |c| c == first || c == '\\')?;
            if self.m.exhausted() {
                return None;
            }
            at += off;
            let before_ok = self
                .src
                .before(&mut self.m, at)
                .is_none_or(|c| !c.is_alphanumeric() || at == from);
            if before_ok {
                let mut scratch = Vec::new();
                if let Some(end) = self.text(&word, at, hi, &mut scratch) {
                    let after_ok = !word.chars().next_back().is_some_and(char::is_alphanumeric)
                        || !self
                            .src
                            .peek(&mut self.m, end, self.src.len())
                            .is_some_and(char::is_alphanumeric);
                    if after_ok {
                        return Some(at);
                    }
                }
            }
            at += ch.len_utf8();
        }
        None
    }

    /// Match `text` at `at`, inside `..hi`. Pushes tokens and returns the end. Source left
    /// over after the last character is a gap.
    fn text(&mut self, text: &str, at: usize, hi: usize, toks: &mut Vec<Tok>) -> Option<usize> {
        let mut p = at;
        let mut run = String::new();
        let mut run_start = at;
        let mut cur = Cur::new(text);
        let flush = |toks: &mut Vec<Tok>, run: &mut String, start: usize, end: usize| {
            if !run.is_empty() {
                push(
                    toks,
                    (TokenKind::Identity, std::mem::take(run), span(start, end)),
                );
            }
        };
        while let Some(ch) = cur.bump(&mut self.m) {
            if self.m.exhausted() {
                return None;
            }
            if ch.is_whitespace() {
                cur.skip_ws(&mut self.m);
                let next = cur.peek(&mut self.m);
                flush(toks, &mut run, run_start, p);
                let begin = p;
                // Table borders sit between the words of a cell.
                while let Some(c) = self.src.peek(&mut self.m, p, hi) {
                    if self.m.exhausted() {
                        return None;
                    }
                    if c.is_whitespace()
                        || (next.is_some() && matches!(c, '|' | '+') && next != Some(c))
                    {
                        p += c.len_utf8();
                    } else {
                        break;
                    }
                }
                if p > begin {
                    push(toks, (TokenKind::Collapse, " ".to_owned(), span(begin, p)));
                } else {
                    let end = toks.last().map_or(begin, |t| t.2.end());
                    push(toks, (TokenKind::Wrap, " ".to_owned(), Span::empty(end)));
                }
                run_start = p;
                continue;
            }
            // `\ ` and `\` + newline vanish from the text.
            let mut q = p;
            loop {
                let c = self.src.peek(&mut self.m, q, hi)?;
                if c != '\\' {
                    break;
                }
                let next = self.src.peek(&mut self.m, q + 1, hi)?;
                if next == ch {
                    break;
                }
                if !next.is_whitespace() {
                    return None;
                }
                self.m.spend(1)?;
                q += 1 + next.len_utf8();
            }
            if q > p {
                flush(toks, &mut run, run_start, p);
                p = q;
                run_start = p;
            }
            let c = self.src.peek(&mut self.m, p, hi)?;
            if c == '\\' {
                // An escape: the backslash and the character it protects.
                let next = self.src.peek(&mut self.m, p + 1, hi)?;
                if next != ch {
                    return None;
                }
                flush(toks, &mut run, run_start, p);
                let end = p + 1 + next.len_utf8();
                push(toks, (TokenKind::Backslash, ch.to_string(), span(p, end)));
                p = end;
                run_start = p;
            } else if c == ch {
                if run.is_empty() {
                    run_start = p;
                }
                run.push(c);
                p += c.len_utf8();
            } else {
                return None;
            }
        }
        flush(toks, &mut run, run_start, p);
        Some(p)
    }
}

/// Push a token. Whitespace that follows whitespace, or opens the list, is dropped.
fn push(toks: &mut Vec<Tok>, tok: Tok) {
    let blank = tok.1.chars().all(char::is_whitespace);
    if blank
        && toks
            .last()
            .is_none_or(|t| t.1.ends_with(char::is_whitespace))
    {
        return;
    }
    toks.push(tok);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::assert_is_empty)]

    /// The most uncharged bytes a run may look at, per byte of input. The charged part is
    /// capped by the budget itself.
    const UNCHARGED_PER_BYTE: usize = 16;

    use super::*;
    use crate::rst::model::Output;

    fn nodes(json: &str) -> Vec<Node> {
        serde_json::from_str::<Output>(&format!(r#"{{"docutils":"0.23","nodes":{json}}}"#))
            .unwrap()
            .nodes
    }

    fn paragraph(start: usize, end: usize, text: &str) -> String {
        format!(
            r#"{{"tag":"paragraph","start":{start},"end":{end},"pieces":[{{"k":"t","t":{}}}]}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    fn first_prose(mapped: &Mapped) -> (String, Span) {
        let block = &mapped.document.blocks[0];
        (
            block.projection.as_ref().unwrap().text().to_owned(),
            block.span,
        )
    }

    #[test]
    fn a_line_off_by_one_is_found_with_slack() {
        // docutils 0.21 reports a rubric one line late: the text is on line 2, not 3.
        let source = "Intro\n\nA rubric\n\nAfter.\n";
        let line_of_rubric = 3;
        let mapped = project(
            source,
            0,
            &nodes(&format!(
                "[{}]",
                paragraph(line_of_rubric + 1, line_of_rubric + 1, "A rubric")
            )),
        );
        let (text, at) = first_prose(&mapped);
        assert_eq!(text, "A rubric");
        assert_eq!(&source[at.start()..at.end()], "A rubric");
        assert!(mapped.notices.is_empty());
    }

    #[test]
    fn text_that_cannot_be_matched_keeps_its_prose_at_line_precision() {
        let source = "One\n\n  Words that docutils changed.  \n\nTwo\n";
        let mapped = project(
            source,
            0,
            &nodes(&format!(
                "[{}]",
                paragraph(3, 3, "Words that docutils rewrote.")
            )),
        );
        let (text, at) = first_prose(&mapped);
        assert_eq!(text, "Words that docutils rewrote.");
        assert_eq!(
            &source[at.start()..at.end()],
            "Words that docutils changed."
        );
        let codes: Vec<_> = mapped.notices.iter().map(|n| (n.code, n.warning)).collect();
        assert_eq!(codes, [("rst-line-precise", false)]);
    }

    #[test]
    fn a_node_without_lines_is_reported_not_dropped_silently() {
        let mapped = project(
            "Text\n",
            0,
            &nodes(
                r#"[{"tag":"classifier","start":null,"end":null,"pieces":[{"k":"t","t":"Text"}]}]"#,
            ),
        );
        assert!(mapped.document.blocks.is_empty());
        assert_eq!(mapped.notices.len(), 1);
        assert_eq!(mapped.notices[0].code, "rst-prose-unplaced");
        assert!(mapped.notices[0].warning);
        assert!(mapped.notices[0].message.contains("classifier"));
    }

    #[test]
    fn a_line_past_the_end_is_reported() {
        let mapped = project(
            "Text\n",
            0,
            &nodes(&format!("[{}]", paragraph(9, 9, "Text"))),
        );
        assert!(mapped.document.blocks.is_empty());
        assert_eq!(mapped.notices[0].code, "rst-prose-unplaced");
    }

    #[test]
    fn an_escaped_space_is_a_gap_inside_a_word() {
        let source = "foo\\ bar\n";
        let mapped = project(
            source,
            0,
            &nodes(&format!("[{}]", paragraph(1, 1, "foobar"))),
        );
        let (text, at) = first_prose(&mapped);
        assert_eq!(text, "foobar");
        assert_eq!(&source[at.start()..at.end()], "foo\\ bar");
        assert!(mapped.notices.is_empty());
    }

    #[test]
    fn crlf_and_lone_cr_end_lines() {
        let source = "one\r\ntwo\rthree\n";
        let mapped = project(
            source,
            0,
            &nodes(&format!(
                "[{},{}]",
                paragraph(2, 2, "two"),
                paragraph(3, 3, "three")
            )),
        );
        let got: Vec<_> = mapped
            .document
            .blocks
            .iter()
            .map(|b| &source[b.span.start()..b.span.end()])
            .collect();
        assert_eq!(got, ["two", "three"]);
    }

    #[test]
    fn a_hostile_prefix_costs_bounded_deterministic_work() {
        // The first word is almost the text on its line: every start position matches for
        // thousands of characters before it fails.
        let run = 4_000;
        let source = format!("{}b\n", "a".repeat(run));
        let word = format!("{}c", "a".repeat(run));
        let mapped = project(&source, 0, &nodes(&format!("[{}]", paragraph(1, 1, &word))));
        let budget = WORK_PER_BYTE * (source.len() + word.len()) + WORK_BASE;
        assert!(
            mapped.work <= budget + run,
            "work {} over budget {budget}",
            mapped.work
        );
        // The prose is kept, at line precision.
        let (text, at) = first_prose(&mapped);
        assert_eq!(text, word);
        assert_eq!(
            &source[at.start()..at.end()],
            format!("{}b", "a".repeat(run))
        );
    }

    #[test]
    fn ordinary_prose_costs_a_few_characters_of_work_per_byte() {
        let mut source = String::new();
        let mut json = Vec::new();
        for i in 0..2_000 {
            let line = format!("Paragraph number {i} has some ordinary words in it.");
            source.push_str(&line);
            source.push_str("\n\n");
            json.push(paragraph(2 * i + 1, 2 * i + 1, &line));
        }
        let mapped = project(&source, 0, &nodes(&format!("[{}]", json.join(","))));
        assert_eq!(mapped.document.blocks.len(), 2_000);
        assert!(mapped.notices.is_empty());
        // Each character is compared once: the work is about twice the text, never more
        // than four times.
        assert!(
            mapped.work <= 4 * source.len(),
            "{} for {}",
            mapped.work,
            source.len()
        );
    }

    #[test]
    fn skipped_escaped_whitespace_is_charged() {
        let pairs = 50_000;
        let source = format!("a{}b\n", "\\ ".repeat(pairs));
        let mapped = project(&source, 0, &nodes(&format!("[{}]", paragraph(1, 1, "ab"))));
        assert!(
            mapped.work >= pairs,
            "only {} charged for {pairs} skipped pairs",
            mapped.work
        );
    }

    #[test]
    fn a_skipped_gap_before_inline_markup_is_charged() {
        let pairs = 50_000;
        let source = format!("a{}*e*\n", "\\ ".repeat(pairs));
        let json = r#"[{"tag":"paragraph","start":1,"end":1,"pieces":[{"k":"t","t":"a"},{"k":"i","t":"e","raw":"*e*"}]}]"#;
        let mapped = project(&source, 0, &nodes(json));
        assert!(mapped.work >= pairs, "only {} charged", mapped.work);
    }

    #[test]
    fn whitespace_runs_in_text_and_in_raw_are_charged() {
        let run = 60_000;
        let source = format!("a{}b\n", " ".repeat(run));
        let mapped = project(&source, 0, &nodes(&format!("[{}]", paragraph(1, 1, "a b"))));
        assert!(mapped.work >= run, "text: only {} charged", mapped.work);
        let source = format!("*e{}f*\n", " ".repeat(run));
        let json = r#"[{"tag":"paragraph","start":1,"end":1,"pieces":[{"k":"i","t":"e f","raw":"*e f*"}]}]"#;
        let mapped = project(&source, 0, &nodes(json));
        assert!(mapped.work >= run, "raw: only {} charged", mapped.work);
    }

    #[test]
    fn a_very_wide_table_exhausts_the_budget_and_keeps_every_cell() {
        // Every cell scans its row up to its column, so the work grows with the square of the
        // width. The budget stops it, and each cell keeps its prose at line precision.
        let columns = 4_000;
        let border = format!("{}+\n", "+-".repeat(columns));
        let row = format!("{}|\n", "|a".repeat(columns));
        let source = format!("{border}{row}{border}");
        let cells: Vec<String> = (0..columns)
            .map(|c| {
                format!(
                    r#"{{"tag":"paragraph","start":2,"end":2,"cell":[0,{c},1],"pieces":[{{"k":"t","t":"a"}}]}}"#
                )
            })
            .collect();
        let mapped = project(&source, 0, &nodes(&format!("[{}]", cells.join(","))));
        let budget = WORK_PER_BYTE * (source.len() + columns) + WORK_BASE;
        assert!(
            mapped.work > budget,
            "the budget was not reached: {} of {budget}",
            mapped.work
        );
        assert!(
            mapped.work <= budget + 2 * source.len(),
            "work {} ran far past {budget}",
            mapped.work
        );
        assert_eq!(mapped.document.blocks.len(), columns);
        assert!(mapped.notices.iter().any(|n| n.code == "rst-line-precise"));
    }

    #[test]
    fn the_border_of_a_table_is_read_once_for_all_its_rows() {
        let rows = 2_000;
        let mut source = String::from("=====  =====\nHead   Head\n=====  =====\n");
        let mut cells = Vec::new();
        for r in 0..rows {
            source.push_str("one    two\n");
            let line = 4 + r;
            for c in 0..2 {
                cells.push(format!(
                    r#"{{"tag":"paragraph","start":{line},"end":{line},"cell":[{},{c},1],"pieces":[{{"k":"t","t":"{}"}}]}}"#,
                    r + 1,
                    ["one", "two"][c]
                ));
            }
        }
        source.push_str("=====  =====\n");
        let mapped = project(&source, 0, &nodes(&format!("[{}]", cells.join(","))));
        assert_eq!(mapped.document.blocks.len(), 2 * rows);
        assert_eq!(mapped.notices, []);
        assert!(
            mapped.work <= 16 * source.len(),
            "{} for {}",
            mapped.work,
            source.len()
        );
    }

    #[test]
    fn an_indented_wide_table_costs_no_rescan_after_the_budget_is_spent() {
        // Every line is indented, so the trimming of a shared row would cost the indent
        // again for each cell.
        let indent = " ".repeat(30_000);
        let columns = 1_500;
        let border = format!("{indent}{}+\n", "+-".repeat(columns));
        let row = format!("{indent}{}|\n", "|a".repeat(columns));
        let source = format!("{border}{row}{border}");
        let cells: Vec<String> = (0..columns)
            .map(|c| {
                format!(
                    r#"{{"tag":"paragraph","start":2,"end":2,"cell":[0,{c},1],"pieces":[{{"k":"t","t":"a"}}]}}"#
                )
            })
            .collect();
        let mapped = project(&source, 0, &nodes(&format!("[{}]", cells.join(","))));
        let size = source.len() + columns;
        let budget = WORK_PER_BYTE * size + WORK_BASE;
        assert!(mapped.work > budget, "the budget was not reached");
        assert!(
            mapped.scanned <= budget + 16 * size,
            "scanned {} bytes for an input of {size}",
            mapped.scanned
        );
        assert_eq!(mapped.document.blocks.len(), columns);
    }

    mod bounded {
        //! Hostile RST shapes: wide, indented and nested tables, comments, escapes and long
        //! prefixes. Whatever the input, the bytes the module looks at stay within a fixed
        //! multiple of the input, charged to the budget or not.

        use proptest::prelude::*;

        use super::*;

        fn line() -> impl Strategy<Value = String> {
            let words = prop::collection::vec("[a-z]{1,8}", 0..12).prop_map(|w| w.join(" "));
            prop_oneof![
                (0..300usize, words.clone())
                    .prop_map(|(indent, text)| format!("{}{text}", " ".repeat(indent))),
                (0..200usize, 1..120usize).prop_map(|(indent, k)| format!(
                    "{}+{}",
                    " ".repeat(indent),
                    "-+".repeat(k)
                )),
                (0..200usize, 1..120usize).prop_map(|(indent, k)| format!(
                    "{}|{}",
                    " ".repeat(indent),
                    "a|".repeat(k)
                )),
                (0..200usize, 1..60usize).prop_map(|(indent, k)| format!(
                    "{}{}",
                    " ".repeat(indent),
                    "====  ".repeat(k)
                )),
                words.prop_map(|w| format!(".. {w}")),
                (1..400usize).prop_map(|n| format!("{}x", "\\ ".repeat(n))),
                (1..2000usize).prop_map(|n| format!("{}b", "a".repeat(n))),
                (1..60usize, 1..3000usize).prop_map(|(k, gap)| format!(
                    ":{}: {}*x",
                    "a".repeat(k),
                    " ".repeat(gap)
                )),
                (1..500usize).prop_map(|k| "*x ".repeat(k)),
                Just(String::new()),
            ]
        }

        fn piece(words: Vec<String>) -> impl Strategy<Value = String> {
            let word = prop::sample::select(words);
            prop_oneof![
                prop::collection::vec(word.clone(), 1..6).prop_map(|w| {
                    format!(
                        r#"{{"k":"t","t":{}}}"#,
                        serde_json::to_string(&w.join(" ")).unwrap()
                    )
                }),
                word.clone().prop_map(|w| {
                    let raw = serde_json::to_string(&format!("*{w}*")).unwrap();
                    format!(
                        r#"{{"k":"i","t":{},"raw":{raw}}}"#,
                        serde_json::to_string(&w).unwrap()
                    )
                }),
                word.prop_map(|w| {
                    let raw = serde_json::to_string(&format!("``{w}``")).unwrap();
                    format!(
                        r#"{{"k":"c","t":{},"raw":{raw}}}"#,
                        serde_json::to_string(&w).unwrap()
                    )
                }),
                (1..3000usize).prop_map(|gap| {
                    let raw = serde_json::to_string(&format!("*x{}y*", " ".repeat(gap))).unwrap();
                    format!(r#"{{"k":"i","t":"x y","raw":{raw}}}"#)
                }),
                Just(r#"{"k":"o","raw":"[1]_"}"#.to_owned()),
            ]
        }

        fn node(lines: usize, words: Vec<String>) -> impl Strategy<Value = String> {
            let top = lines + 2; // Sometimes past the end.
            (
                1..top,
                0..4usize,
                prop::option::of((0..4usize, 0..60usize, 1..4usize)),
                prop::bool::weighted(0.1),
                prop::collection::vec(piece(words), 1..5),
            )
                .prop_map(|(start, extra, cell, table, pieces)| {
                    let end = start + extra;
                    let cell = cell
                        .map(|(r, c, n)| format!(r#","cell":[{r},{c},{n}]"#))
                        .unwrap_or_default();
                    if table {
                        format!(r#"{{"tag":"table","table":true,"start":{start},"end":{end}}}"#)
                    } else {
                        format!(
                            r#"{{"tag":"paragraph","start":{start},"end":{end}{cell},"pieces":[{}]}}"#,
                            pieces.join(",")
                        )
                    }
                })
        }

        fn case() -> impl Strategy<Value = (String, String)> {
            prop::collection::vec(line(), 1..40).prop_flat_map(|lines| {
                let eol = prop::sample::select(vec!["\n", "\r\n", "\r"]);
                let words: Vec<String> = lines
                    .iter()
                    .flat_map(|l| l.split_whitespace().map(str::to_owned))
                    .chain(["alpha".to_owned()])
                    .map(|w| w.chars().take(40).collect())
                    .collect();
                let count = lines.len();
                (
                    Just(lines),
                    eol,
                    prop::collection::vec(node(count, words), 1..60),
                )
                    .prop_map(|(lines, eol, nodes)| {
                        (lines.join(eol) + eol, format!("[{}]", nodes.join(",")))
                    })
            })
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(48))]

            #[test]
            fn the_bytes_looked_at_stay_a_fixed_multiple_of_the_input((source, json) in case()) {
                let parsed = nodes(&json);
                let mapped = project(&source, 0, &parsed);
                let text: usize = parsed.iter().map(|n| n.pieces.iter().map(|p| p.t.len() + p.raw.len()).sum::<usize>()).sum();
                let size = source.len() + text;
                let budget = WORK_PER_BYTE * size + WORK_BASE;
                prop_assert!(
                    mapped.scanned <= budget + UNCHARGED_PER_BYTE * (size + 1),
                    "scanned {} for an input of {size} (budget {budget})",
                    mapped.scanned
                );
            }
        }
    }

    #[test]
    fn the_scans_of_a_cells_own_view_are_counted() {
        let source = "+--+--+\n|ab|cd|\n+--+--+\n";
        let cells = r#"[
            {"tag":"paragraph","start":2,"end":2,"cell":[0,0,1],"pieces":[{"k":"t","t":"ab"}]},
            {"tag":"paragraph","start":2,"end":2,"cell":[0,1,1],"pieces":[{"k":"t","t":"cd"}]}]"#;
        let mapped = project(source, 0, &nodes(cells));
        assert_eq!(mapped.document.blocks.len(), 2);
        assert!(
            mapped.scanned >= mapped.work,
            "scanned {} is below the {} charged",
            mapped.scanned,
            mapped.work
        );
    }

    #[test]
    fn long_whitespace_in_an_inline_construct_is_not_rescanned_at_every_near_match() {
        let near_matches = 2_000;
        let gap = 20_000;
        let source = format!("{}\n", "*x ".repeat(near_matches));
        let raw = format!("*x{}y*", " ".repeat(gap));
        let json = format!(
            r#"[{{"tag":"paragraph","start":1,"end":1,"pieces":[{{"k":"i","t":"x y","raw":{}}}]}}]"#,
            serde_json::to_string(&raw).unwrap()
        );
        let mapped = project(&source, 0, &nodes(&json));
        let size = source.len() + raw.len() + 3;
        let budget = WORK_PER_BYTE * size + WORK_BASE;
        assert!(
            mapped.scanned <= budget + UNCHARGED_PER_BYTE * size,
            "scanned {} for an input of {size}",
            mapped.scanned
        );
        assert_eq!(mapped.document.blocks.len(), 1);
    }
}
