//! YAML front matter, parsed with `saphyr-parser` events.
//!
//! The tree is built here from the event stream rather than by saphyr's recursive loader, so
//! nesting depth and alias expansion are bounded before any recursion or cloning happens.

use std::collections::{HashMap, HashSet};

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
    Collection,
    Plain,
    Quoted(u8),
}

impl Offsets<'_> {
    fn byte(&self, char_index: usize) -> usize {
        self.base
            + match &self.chars {
                None => char_index.min(self.content.len()),
                Some(t) => t[char_index.min(t.len() - 1)],
            }
    }

    fn caret(&self, at: Marker) -> crate::Span {
        let at = self.byte(at.index());
        self.index.span(at..at)
    }

    /// The span of a key or value as written.
    ///
    /// saphyr starts a node at its anchor or tag, ends a flow collection before its closing
    /// bracket, keeps the trailing newline on block nodes, and runs a quoted scalar on through
    /// a trailing comment. This looks past the properties to the node's first byte and then
    /// fixes the end.
    fn span(&self, start: Marker, end: Marker, kind: Kind) -> crate::Span {
        let start = self.byte(start.index());
        let end = self.byte(end.index()).max(start);
        let bytes = self.content.as_bytes();
        let (lo, mut hi) = (start - self.base, end - self.base);
        let mut first = lo;
        while matches!(bytes.get(first), Some(b'&' | b'!')) {
            while bytes.get(first).is_some_and(|b| !b.is_ascii_whitespace()) {
                first += 1;
            }
            while bytes.get(first).is_some_and(|b| matches!(b, b' ' | b'\t')) {
                first += 1;
            }
        }
        match (kind, bytes.get(first)) {
            (Kind::Collection, Some(b'[')) if bytes.get(hi) == Some(&b']') => hi += 1,
            (Kind::Collection, Some(b'{')) if bytes.get(hi) == Some(&b'}') => hi += 1,
            (Kind::Quoted(q), Some(&b)) if b == q => {
                if let Some(close) = closing_quote(bytes, first, q, hi) {
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
        self.index.span(start..self.base + hi.max(lo))
    }
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

/// Approximate heap cost of a node, charged against the alias budget.
const NODE_COST: usize = 16;

enum Frame {
    Seq {
        start: Marker,
        anchor: usize,
        items: Vec<Node>,
        size: usize,
    },
    Map {
        start: Marker,
        anchor: usize,
        entries: Vec<Entry>,
        keys: HashSet<String>,
        key: Option<(String, crate::Span)>,
        size: usize,
    },
}

struct Builder<'a> {
    offsets: Offsets<'a>,
    limits: &'a Limits,
    stack: Vec<Frame>,
    root: Option<Node>,
    documents: usize,
    anchors: HashMap<usize, (Node, usize)>,
    alias_spent: usize,
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
    let offsets = Offsets {
        content,
        base,
        chars,
        index,
    };
    let mut b = Builder {
        offsets,
        limits,
        stack: Vec::new(),
        root: None,
        documents: 0,
        anchors: HashMap::new(),
        alias_spent: 0,
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
        fail(e.info(), Some(self.offsets.caret(*e.marker())))
    }

    fn event(&mut self, event: Event<'_>, start: Marker, end: Marker) -> Res<()> {
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
            Event::Scalar(text, style, anchor, tag) => {
                let kind = match style {
                    ScalarStyle::SingleQuoted => Kind::Quoted(b'\''),
                    ScalarStyle::DoubleQuoted => Kind::Quoted(b'"'),
                    _ => Kind::Plain,
                };
                let span = self.offsets.span(start, end, kind);
                if self.expecting_key() {
                    let node = Node {
                        value: Value::String(text.to_string()),
                        span: span.clone(),
                    };
                    self.register(anchor, &node, NODE_COST + text.len());
                    return self.set_key(text.to_string(), span);
                }
                let size = NODE_COST + text.len();
                let value = match Scalar::parse_from_cow_and_metadata(text, style, tag.as_ref()) {
                    Some(Scalar::Null) => Value::Null,
                    Some(Scalar::Boolean(v)) => Value::Bool(v),
                    Some(Scalar::Integer(v)) => Value::Integer(v),
                    Some(Scalar::FloatingPoint(v)) => Value::Float(v.into_inner()),
                    Some(Scalar::String(s)) => Value::String(s.to_string()),
                    None => return Err(fail("the value does not match its tag", Some(span))),
                };
                self.complete(Node { value, span }, anchor, size)?;
            }
            Event::Alias(id) => self.alias(id, start, end)?,
            Event::SequenceStart(anchor, _) | Event::MappingStart(anchor, _) => {
                let is_seq = matches!(event, Event::SequenceStart(..));
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
                        anchor,
                        items: Vec::new(),
                        size: NODE_COST,
                    }
                } else {
                    Frame::Map {
                        start,
                        anchor,
                        entries: Vec::new(),
                        keys: HashSet::new(),
                        key: None,
                        size: NODE_COST,
                    }
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let Some(frame) = self.stack.pop() else {
                    return Err(fail("unbalanced collection", Some(self.offsets.caret(end))));
                };
                let (start, anchor, value, size) = match frame {
                    Frame::Seq {
                        start,
                        anchor,
                        items,
                        size,
                    } => (start, anchor, Value::Array(items), size),
                    Frame::Map {
                        start,
                        anchor,
                        entries,
                        size,
                        ..
                    } => (start, anchor, Value::Table(entries), size),
                };
                let span = self.offsets.span(start, end, Kind::Collection);
                self.complete(Node { value, span }, anchor, size)?;
            }
        }
        Ok(())
    }

    fn expecting_key(&self) -> bool {
        matches!(self.stack.last(), Some(Frame::Map { key: None, .. }))
    }

    fn set_key(&mut self, text: String, span: crate::Span) -> Res<()> {
        let Some(Frame::Map {
            keys, key, size, ..
        }) = self.stack.last_mut()
        else {
            unreachable!("only called while a mapping expects a key");
        };
        if !keys.insert(text.clone()) {
            return Err(fail(format!("duplicate key `{text}`"), Some(span)));
        }
        *size += text.len();
        *key = Some((text, span));
        Ok(())
    }

    fn register(&mut self, anchor: usize, node: &Node, size: usize) {
        if anchor > 0 {
            self.anchors.insert(anchor, (node.clone(), size));
        }
    }

    fn alias(&mut self, id: usize, start: Marker, end: Marker) -> Res<()> {
        let span = self.offsets.span(start, end, Kind::Plain);
        let Some((node, size)) = self.anchors.get(&id) else {
            return Err(fail("alias to an unknown anchor", Some(span)));
        };
        self.alias_spent = self.alias_spent.saturating_add(*size);
        if self.alias_spent > self.limits.max_alias_bytes {
            return Err(fail(
                format!(
                    "alias expansion is over the limit of {} bytes",
                    self.limits.max_alias_bytes
                ),
                Some(span),
            ));
        }
        let size = *size;
        let mut node = node.clone();
        if self.expecting_key() {
            let text = match &node.value {
                Value::String(s) => s.clone(),
                Value::Integer(i) => i.to_string(),
                Value::Float(f) => f.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => "null".to_owned(),
                _ => {
                    return Err(fail(
                        "a non-scalar key is not supported in front matter",
                        Some(span),
                    ));
                }
            };
            return self.set_key(text, span);
        }
        node.span = span;
        self.complete(node, 0, size)
    }

    /// Hand a finished node to its parent, or make it the root.
    fn complete(&mut self, node: Node, anchor: usize, size: usize) -> Res<()> {
        self.register(anchor, &node, size);
        match self.stack.last_mut() {
            None => self.root = Some(node),
            Some(Frame::Seq {
                items, size: total, ..
            }) => {
                *total += size;
                items.push(node);
            }
            Some(Frame::Map {
                entries,
                key,
                size: total,
                ..
            }) => {
                let Some((key, key_span)) = key.take() else {
                    return Err(fail("a value has no key", Some(node.span)));
                };
                *total += size;
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
