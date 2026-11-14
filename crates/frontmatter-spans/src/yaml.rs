//! YAML front matter, parsed with `saphyr-parser` events.
//!
//! The tree is built here from the event stream rather than by saphyr's recursive loader, so
//! nesting depth is bounded before anything recurses.
//!
//! Like `StrictYAML`, this reader refuses anchors, aliases and explicit tags. Front matter is
//! small, human-written metadata; those features add expansion attacks and type surprises.
//! Mapping keys are always strings, exactly as written.

use std::collections::HashSet;

use saphyr::Scalar;
use saphyr_parser::{Event, Marker, Parser, ScalarStyle, ScanError};

use crate::{Entry, Format, Limits, LineIndex, Node, ParseError, Value};

/// Turns saphyr's character indexes into whole-document byte offsets.
struct Offsets<'a> {
    content: &'a str,
    base: usize,
    /// Byte offset of every character, plus the end. `None` when the text is ASCII.
    chars: Option<Vec<usize>>,
    index: &'a LineIndex<'a>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Quoted(u8),
}

impl Offsets<'_> {
    /// Byte offset in the content (not the document) of a marker.
    fn local(&self, at: Marker) -> usize {
        match &self.chars {
            None => at.index().min(self.content.len()),
            Some(t) => t[at.index().min(t.len() - 1)],
        }
    }

    fn caret(&self, at: Marker) -> crate::Span {
        let at = self.base + self.local(at);
        self.index.span(at..at)
    }

    /// The span of a scalar (or a rejected alias) as written.
    ///
    /// saphyr keeps the trailing newline on block scalars and runs a quoted scalar on through a
    /// trailing comment. This fixes the end. Nodes have no anchor or tag, so the start is the
    /// node's first byte.
    fn span(&self, start: Marker, end: Marker, kind: Kind) -> crate::Span {
        let lo = self.local(start);
        let mut hi = self.local(end).max(lo);
        let bytes = self.content.as_bytes();
        match (kind, bytes.get(lo)) {
            (Kind::Quoted(q), Some(&b)) if b == q => {
                if let Some(close) = closing_quote(bytes, lo, q, hi) {
                    hi = close + 1;
                }
            }
            _ => {}
        }
        if !matches!(kind, Kind::Quoted(_)) {
            while hi > lo && bytes[hi - 1].is_ascii_whitespace() {
                hi -= 1;
            }
        }
        self.index.span(self.base + lo..self.base + hi)
    }

    /// The span of an omitted value: zero-width, right after the `:` or `-` that introduces it.
    ///
    /// saphyr puts the omitted value at the indicator or, when a comment follows, at the end of
    /// the line. This walks back over blanks, newlines and comments to the indicator.
    fn empty_value(&self, at: Marker) -> crate::Span {
        let bytes = self.content.as_bytes();
        let lo = self.local(at);
        let pos = if matches!(bytes.get(lo), Some(b':' | b'-')) {
            lo + 1
        } else {
            self.indicator_end(lo)
                .filter(|&e| matches!(bytes[e - 1], b':' | b'-'))
                .unwrap_or(lo)
        };
        let pos = self.base + pos;
        self.index.span(pos..pos)
    }

    /// The content offset of the `-` that introduces the entry starting at document offset
    /// `entry_start`, if one precedes it.
    fn dash_before(&self, entry_start: usize) -> Option<usize> {
        let end = self.indicator_end(entry_start - self.base)?;
        (self.content.as_bytes()[end - 1] == b'-').then(|| end - 1)
    }

    /// The offset just past the last token before `pos`, skipping blanks, newlines and comments.
    fn indicator_end(&self, pos: usize) -> Option<usize> {
        let bytes = self.content.as_bytes();
        let mut end = pos;
        loop {
            while end > 0 && bytes[end - 1].is_ascii_whitespace() {
                end -= 1;
            }
            if end == 0 {
                return None;
            }
            let line_start = bytes[..end]
                .iter()
                .rposition(|b| matches!(b, b'\n' | b'\r'))
                .map_or(0, |i| i + 1);
            match comment_start(&bytes[line_start..end]) {
                Some(c) => end = line_start + c,
                None => return Some(end),
            }
        }
    }

    /// The span of a sequence or mapping.
    ///
    /// A flow collection ends one byte after its closing delimiter, found from the start of
    /// saphyr's closing event. A block collection ends where its last entry ends, so trailing
    /// comments and blank lines stay outside.
    fn collection_span(
        &self,
        open: Marker,
        close: Marker,
        first_dash: Option<usize>,
        last_child_end: Option<usize>,
    ) -> crate::Span {
        let bytes = self.content.as_bytes();
        // saphyr starts a block sequence after its first dash in some layouts (an omitted first
        // entry, or dashes at the parent key's indent), so take the dash from the first entry.
        let lo = first_dash.unwrap_or_else(|| self.local(open));
        let closer = match bytes.get(lo) {
            Some(b'[') => Some(b']'),
            Some(b'{') => Some(b'}'),
            _ => None,
        };
        let hi = closer
            .and_then(|c| self.closing_delimiter(self.local(close), c))
            .map_or_else(|| last_child_end.map_or(lo, |e| e - self.base), |at| at + 1);
        self.index.span(self.base + lo..self.base + hi.max(lo))
    }

    /// Where the closing `delimiter` of a flow collection is, given the start of saphyr's
    /// closing event. After a trailing comma saphyr reports the comma, so look past commas,
    /// blanks, newlines and comments.
    fn closing_delimiter(&self, from: usize, delimiter: u8) -> Option<usize> {
        let bytes = self.content.as_bytes();
        let mut i = from;
        while i < bytes.len() {
            match bytes[i] {
                b if b == delimiter => return Some(i),
                b',' | b' ' | b'\t' | b'\n' | b'\r' => i += 1,
                b'#' => {
                    while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                        i += 1;
                    }
                }
                _ => return None,
            }
        }
        None
    }

    /// The `*name` token around a scan error, when the error is about an alias.
    fn alias_token(&self, at: Marker) -> Option<crate::Span> {
        let bytes = self.content.as_bytes();
        let is_name =
            |b: u8| !b.is_ascii_whitespace() && !matches!(b, b',' | b'[' | b']' | b'{' | b'}');
        let mut lo = self.local(at);
        if bytes.get(lo) != Some(&b'*') {
            while lo > 0 && is_name(bytes[lo - 1]) && bytes[lo - 1] != b'*' {
                lo -= 1;
            }
            lo = lo.checked_sub(1).filter(|&i| bytes[i] == b'*')?;
        }
        let mut hi = lo + 1;
        while hi < bytes.len() && is_name(bytes[hi]) {
            hi += 1;
        }
        Some(self.index.span(self.base + lo..self.base + hi))
    }

    /// The span of the first `&` anchor or `!` tag token that belongs to the node starting at
    /// `start`, whether it sits on the node's line or on a line before it.
    fn property(&self, start: Marker, sigil: u8) -> Option<crate::Span> {
        let bytes = self.content.as_bytes();
        let at = self.local(start);
        let is_ws = |i: usize| bytes[i].is_ascii_whitespace();
        let token_end = |mut i: usize| {
            if bytes[i] == b'!' && bytes.get(i + 1) == Some(&b'<') {
                // A verbatim tag runs to its closing `>` and may contain commas.
                while i < bytes.len() && bytes[i] != b'>' && !is_ws(i) {
                    i += 1;
                }
                return (i + 1).min(bytes.len());
            }
            while i < bytes.len() && !is_ws(i) && !matches!(bytes[i], b',' | b']' | b'}') {
                i += 1;
            }
            i
        };
        let mut tokens: Vec<(usize, usize)> = Vec::new();
        if matches!(bytes.get(at), Some(b'&' | b'!')) {
            let mut i = at;
            while matches!(bytes.get(i), Some(b'&' | b'!')) {
                let end = token_end(i);
                tokens.push((i, end));
                i = end;
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                    i += 1;
                }
            }
        } else {
            let mut pos = at;
            loop {
                while pos > 0 && is_ws(pos - 1) {
                    pos -= 1;
                }
                let end = pos;
                while pos > 0 && !is_ws(pos - 1) {
                    pos -= 1;
                }
                if pos < end && matches!(bytes[pos], b'&' | b'!') {
                    tokens.push((pos, end));
                } else {
                    break;
                }
            }
            tokens.reverse();
        }
        tokens
            .into_iter()
            .find(|&(s, _)| bytes[s] == sigil)
            .map(|(s, e)| self.index.span(self.base + s..self.base + e))
    }
}

/// The index of a `#` that starts a comment in `line`: outside quotes, at the start or after a blank.
fn comment_start(line: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < line.len() {
        match line[i] {
            b'"' => {
                i += 1;
                while i < line.len() && line[i] != b'"' {
                    i += if line[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'\'' => {
                i += 1;
                while i < line.len() {
                    if line[i] == b'\'' {
                        if line.get(i + 1) == Some(&b'\'') {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    i += 1;
                }
            }
            b'#' if i == 0 || matches!(line[i - 1], b' ' | b'\t') => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The index of the quote that closes the scalar opened at `open`, searching up to `limit`.
fn closing_quote(bytes: &[u8], open: usize, quote: u8, limit: usize) -> Option<usize> {
    let mut i = open + 1;
    while i < limit.min(bytes.len()) {
        match bytes[i] {
            b'\\' if quote == b'"' => i += 1,
            b if b == quote => {
                if quote == b'\'' && bytes.get(i + 1) == Some(&b'\'') {
                    i += 1;
                } else {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

enum Frame {
    Seq {
        start: Marker,
        items: Vec<Node>,
    },
    Map {
        start: Marker,
        entries: Vec<Entry>,
        keys: HashSet<String>,
        key: Option<(String, crate::Span)>,
    },
}

struct Builder<'a> {
    offsets: Offsets<'a>,
    limits: &'a Limits,
    stack: Vec<Frame>,
    root: Option<Node>,
    documents: usize,
}

type Res<T> = Result<T, ParseError>;

fn fail(message: impl Into<String>, span: Option<crate::Span>) -> ParseError {
    ParseError {
        format: Format::Yaml,
        message: message.into(),
        span,
    }
}

pub(crate) fn parse(
    content: &str,
    base: usize,
    index: &LineIndex<'_>,
    limits: &Limits,
) -> Res<Node> {
    let chars = (!content.is_ascii()).then(|| {
        content
            .char_indices()
            .map(|(i, _)| i)
            .chain([content.len()])
            .collect()
    });
    let mut b = Builder {
        offsets: Offsets {
            content,
            base,
            chars,
            index,
        },
        limits,
        stack: Vec::new(),
        root: None,
        documents: 0,
    };
    for item in Parser::new_from_str(content) {
        let (event, span) = item.map_err(|e| b.scan_error(&e))?;
        b.event(event, span.start, span.end)?;
    }
    Ok(b.root.unwrap_or_else(|| Node {
        value: Value::Null,
        span: index.span(base..base + content.len()),
    }))
}

impl Builder<'_> {
    fn scan_error(&self, e: &ScanError) -> ParseError {
        let span = if e.info().contains("anchor") {
            self.offsets.alias_token(*e.marker())
        } else {
            None
        };
        fail(
            e.info(),
            Some(span.unwrap_or_else(|| self.offsets.caret(*e.marker()))),
        )
    }

    /// Refuse an anchor or tag on the node that starts at `start`.
    fn check_properties(&self, anchor: usize, has_tag: bool, start: Marker) -> Res<()> {
        for (present, sigil, what) in [(anchor > 0, b'&', "anchor"), (has_tag, b'!', "tag")] {
            if present {
                let span = self
                    .offsets
                    .property(start, sigil)
                    .unwrap_or_else(|| self.offsets.caret(start));
                return Err(fail(
                    format!("a YAML {what} is not supported in front matter"),
                    Some(span),
                ));
            }
        }
        Ok(())
    }

    fn event(&mut self, event: Event<'_>, start: Marker, end: Marker) -> Res<()> {
        let is_seq = matches!(event, Event::SequenceStart(..));
        match event {
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {}
            Event::DocumentStart(_) => {
                self.documents += 1;
                if self.documents > 1 {
                    return Err(fail(
                        "the block holds more than one YAML document",
                        Some(self.offsets.caret(start)),
                    ));
                }
            }
            Event::Alias(_) => {
                return Err(fail(
                    "a YAML alias is not supported in front matter",
                    Some(self.offsets.span(start, end, Kind::Plain)),
                ));
            }
            Event::Scalar(text, style, anchor, tag) => {
                self.check_properties(anchor, tag.is_some(), start)?;
                let kind = match style {
                    ScalarStyle::SingleQuoted => Kind::Quoted(b'\''),
                    ScalarStyle::DoubleQuoted => Kind::Quoted(b'"'),
                    _ => Kind::Plain,
                };
                let span = if text.is_empty() && kind == Kind::Plain {
                    self.offsets.empty_value(start)
                } else {
                    self.offsets.span(start, end, kind)
                };
                if self.expecting_key() {
                    return self.set_key(text.to_string(), span);
                }
                let value = match Scalar::parse_from_cow_and_metadata(text, style, None) {
                    Some(Scalar::Null) | None => Value::Null,
                    Some(Scalar::Boolean(v)) => Value::Bool(v),
                    Some(Scalar::Integer(v)) => Value::Integer(v),
                    Some(Scalar::FloatingPoint(v)) => Value::Float(v.into_inner()),
                    Some(Scalar::String(s)) => Value::String(s.to_string()),
                };
                self.complete(Node { value, span })?;
            }
            Event::SequenceStart(anchor, tag) | Event::MappingStart(anchor, tag) => {
                self.check_properties(anchor, tag.is_some(), start)?;
                if self.expecting_key() {
                    return Err(fail(
                        "a non-scalar key is not supported in front matter",
                        Some(self.offsets.caret(start)),
                    ));
                }
                if self.stack.len() >= self.limits.max_depth {
                    return Err(fail(
                        format!("nesting is too deep (limit {})", self.limits.max_depth),
                        Some(self.offsets.caret(start)),
                    ));
                }
                self.stack.push(if is_seq {
                    Frame::Seq {
                        start,
                        items: Vec::new(),
                    }
                } else {
                    Frame::Map {
                        start,
                        entries: Vec::new(),
                        keys: HashSet::new(),
                        key: None,
                    }
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let Some(frame) = self.stack.pop() else {
                    return Err(fail("unbalanced collection", Some(self.offsets.caret(end))));
                };
                let (open, dash, last_end, value) = match frame {
                    Frame::Seq { start, items } => {
                        let last = items.last().map(|n| n.span.range.end);
                        let dash = items
                            .first()
                            .and_then(|n| self.offsets.dash_before(n.span.range.start));
                        (start, dash, last, Value::Array(items))
                    }
                    Frame::Map { start, entries, .. } => {
                        let last = entries
                            .last()
                            .map(|e| e.value.span.range.end.max(e.key_span.range.end));
                        (start, None, last, Value::Table(entries))
                    }
                };
                let span = self.offsets.collection_span(open, start, dash, last_end);
                self.complete(Node { value, span })?;
            }
        }
        Ok(())
    }

    fn expecting_key(&self) -> bool {
        matches!(self.stack.last(), Some(Frame::Map { key: None, .. }))
    }

    fn set_key(&mut self, text: String, span: crate::Span) -> Res<()> {
        let Some(Frame::Map { keys, key, .. }) = self.stack.last_mut() else {
            unreachable!("only called while a mapping expects a key");
        };
        if !keys.insert(text.clone()) {
            return Err(fail(format!("duplicate key `{text}`"), Some(span)));
        }
        *key = Some((text, span));
        Ok(())
    }

    /// Hand a finished node to its parent, or make it the root.
    fn complete(&mut self, node: Node) -> Res<()> {
        match self.stack.last_mut() {
            None => self.root = Some(node),
            Some(Frame::Seq { items, .. }) => items.push(node),
            Some(Frame::Map { entries, key, .. }) => {
                let Some((key, key_span)) = key.take() else {
                    return Err(fail("a value has no key", Some(node.span)));
                };
                entries.push(Entry {
                    key,
                    key_span,
                    value: node,
                });
            }
        }
        Ok(())
    }
}
