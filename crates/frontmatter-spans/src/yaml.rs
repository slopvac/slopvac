//! YAML front matter, parsed with saphyr.

use saphyr::{LoadableYamlNode, MarkedYaml, Marker, Scalar, YamlData};

use crate::{Entry, Format, LineIndex, Node, ParseError, Value};

/// Turns saphyr's character indexes into whole-document byte offsets.
struct Offsets<'a> {
    content: &'a str,
    base: usize,
    /// Byte offset of every character, plus the end. `None` when the text is ASCII.
    chars: Option<Vec<usize>>,
    index: &'a LineIndex<'a>,
}

impl Offsets<'_> {
    fn byte(&self, char_index: usize) -> usize {
        self.base
            + match &self.chars {
                None => char_index,
                Some(t) => t[char_index.min(t.len() - 1)],
            }
    }

    /// A span for a key or a value. `collection` is true for sequences and mappings.
    ///
    /// saphyr ends a flow collection before its closing bracket and keeps the trailing
    /// newline on block collections and block scalars. This puts the closer inside the span
    /// and drops trailing whitespace.
    fn span(&self, start: Marker, end: Marker, collection: bool) -> crate::Span {
        let start = self.byte(start.index());
        let end = self.byte(end.index()).max(start);
        let (lo, mut hi) = (start - self.base, end - self.base);
        let bytes = self.content.as_bytes();
        let closer = match bytes.get(lo) {
            Some(b'[') => Some(b']'),
            Some(b'{') => Some(b'}'),
            _ => None,
        };
        if collection && closer.is_some() && bytes.get(hi) == closer.as_ref() {
            hi += 1;
        }
        while hi > lo && bytes[hi - 1].is_ascii_whitespace() {
            hi -= 1;
        }
        self.index.span(start..self.base + hi)
    }

    fn plain_span(&self, start: Marker, end: Marker) -> crate::Span {
        self.span(start, end, false)
    }
}

pub(crate) fn parse(content: &str, base: usize, index: &LineIndex<'_>) -> Result<Node, ParseError> {
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
    let fail = |message: String, span: Option<crate::Span>| ParseError {
        format: Format::Yaml,
        message,
        span,
    };
    let mut docs = MarkedYaml::load_from_str(content).map_err(|e| {
        let at = offsets.byte(e.marker().index());
        fail(e.info().to_owned(), Some(index.span(at..at)))
    })?;
    match docs.len() {
        0 => Ok(Node {
            value: Value::Null,
            span: index.span(base..base + content.len()),
        }),
        1 => convert(&docs.remove(0), &offsets),
        _ => Err(fail(
            "the block holds more than one YAML document".to_owned(),
            Some(offsets.plain_span(docs[1].span.start, docs[1].span.end)),
        )),
    }
}

fn convert(node: &MarkedYaml<'_>, offsets: &Offsets<'_>) -> Result<Node, ParseError> {
    let collection = matches!(node.data, YamlData::Sequence(_) | YamlData::Mapping(_));
    let span = offsets.span(node.span.start, node.span.end, collection);
    let unsupported = |what: &str| ParseError {
        format: Format::Yaml,
        message: format!("{what} is not supported in front matter"),
        span: Some(span.clone()),
    };
    let value = match &node.data {
        YamlData::Value(scalar) => match scalar {
            Scalar::Null => Value::Null,
            Scalar::Boolean(b) => Value::Bool(*b),
            Scalar::Integer(i) => Value::Integer(*i),
            Scalar::FloatingPoint(f) => Value::Float(f.into_inner()),
            Scalar::String(s) => Value::String(s.to_string()),
        },
        YamlData::Representation(s, _, _) => Value::String(s.to_string()),
        YamlData::Tagged(_, inner) => return convert(inner, offsets),
        YamlData::Sequence(items) => Value::Array(
            items
                .iter()
                .map(|n| convert(n, offsets))
                .collect::<Result<_, _>>()?,
        ),
        YamlData::Mapping(map) => Value::Table(
            map.iter()
                .map(|(k, v)| {
                    Ok(Entry {
                        key: key_text(k).ok_or_else(|| unsupported("a non-scalar key"))?,
                        key_span: offsets.plain_span(k.span.start, k.span.end),
                        value: convert(v, offsets)?,
                    })
                })
                .collect::<Result<_, ParseError>>()?,
        ),
        YamlData::Alias(_) => return Err(unsupported("an alias")),
        YamlData::BadValue => return Err(unsupported("this value")),
    };
    Ok(Node { value, span })
}

fn key_text(key: &MarkedYaml<'_>) -> Option<String> {
    match &key.data {
        YamlData::Value(Scalar::Null) => Some("null".to_owned()),
        YamlData::Value(Scalar::Boolean(b)) => Some(b.to_string()),
        YamlData::Value(Scalar::Integer(i)) => Some(i.to_string()),
        YamlData::Value(Scalar::FloatingPoint(f)) => Some(f.to_string()),
        YamlData::Value(Scalar::String(s)) | YamlData::Representation(s, _, _) => {
            Some(s.to_string())
        }
        YamlData::Tagged(_, inner) => key_text(inner),
        _ => None,
    }
}
