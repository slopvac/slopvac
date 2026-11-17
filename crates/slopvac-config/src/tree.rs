//! A parsed document with a byte span on every key and value.
//!
//! TOML (`toml_edit`) and YAML (`saphyr`) parse into the same tree, so
//! validation, merging and diagnostics never depend on the source format.

use std::ops::Range;

use saphyr::{LoadableYamlNode, MarkedYaml, Scalar, YamlData};
use serde_json::{Map, Number, Value as Json};

/// A parsed value and the byte range of the text that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Byte range of the value in the source text.
    pub span: Range<usize>,
    /// The value.
    pub value: Value,
}

/// One key of a table.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The key text.
    pub key: String,
    /// Byte range of the key in the source text.
    pub key_span: Range<usize>,
    /// The value under the key.
    pub node: Node,
}

/// A parsed value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// YAML `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A floating-point number.
    Float(f64),
    /// A string.
    Str(String),
    /// A TOML date-time, kept as text. It is never a string.
    Datetime(String),
    /// A sequence.
    Array(Vec<Node>),
    /// A mapping, in source order.
    Table(Vec<Entry>),
}

impl Node {
    /// The entry for `key`, when this node is a table that has it.
    #[must_use]
    pub fn entry(&self, key: &str) -> Option<&Entry> {
        match &self.value {
            Value::Table(entries) => entries.iter().find(|e| e.key == key),
            _ => None,
        }
    }

    /// The value under `key`, when this node is a table that has it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.entry(key).map(|e| &e.node)
    }

    /// A short name of the value kind, for messages.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self.value {
            Value::Null => "null",
            Value::Bool(_) => "a boolean",
            Value::Int(_) => "an integer",
            Value::Float(_) => "a float",
            Value::Str(_) => "a string",
            Value::Datetime(_) => "a date-time",
            Value::Array(_) => "an array",
            Value::Table(_) => "a table",
        }
    }

    /// The string, when this node is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The value as JSON, without spans. A non-finite float becomes `null`; validation rejects
    /// those before any caller sees the JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match &self.value {
            Value::Null => Json::Null,
            Value::Bool(b) => Json::Bool(*b),
            Value::Int(i) => Json::Number((*i).into()),
            Value::Float(f) => Number::from_f64(*f).map_or(Json::Null, Json::Number),
            Value::Str(s) | Value::Datetime(s) => Json::String(s.clone()),
            Value::Array(items) => Json::Array(items.iter().map(Node::to_json).collect()),
            Value::Table(entries) => {
                let mut map = Map::with_capacity(entries.len());
                for e in entries {
                    map.insert(e.key.clone(), e.node.to_json());
                }
                Json::Object(map)
            }
        }
    }

    /// Move every span by `offset` bytes, for a block cut out of a larger document.
    pub fn shift(&mut self, offset: usize) {
        self.span = shifted(&self.span, offset);
        match &mut self.value {
            Value::Array(items) => items.iter_mut().for_each(|n| n.shift(offset)),
            Value::Table(entries) => {
                for e in entries {
                    e.key_span = shifted(&e.key_span, offset);
                    e.node.shift(offset);
                }
            }
            _ => {}
        }
    }
}

fn shifted(r: &Range<usize>, offset: usize) -> Range<usize> {
    r.start + offset..r.end + offset
}

/// Why a document did not parse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ParseError {
    /// The parser's message.
    pub message: String,
    /// Where the parser stopped, when it says.
    pub span: Option<Range<usize>>,
}

/// Parse a TOML document.
pub fn parse_toml(text: &str) -> Result<Node, ParseError> {
    let doc = toml_edit::Document::parse(text).map_err(|e| ParseError {
        message: e.message().to_owned(),
        span: e.span(),
    })?;
    Ok(Node {
        span: 0..text.len(),
        value: toml_table(doc.as_table(), &(0..text.len())),
    })
}

fn toml_table(table: &toml_edit::Table, fallback: &Range<usize>) -> Value {
    let mut entries = Vec::new();
    for (key, item) in table {
        let key_span = table
            .key(key)
            .and_then(toml_edit::Key::span)
            .unwrap_or_else(|| fallback.clone());
        entries.push(Entry {
            key: key.to_owned(),
            node: toml_item(item, &key_span),
            key_span,
        });
    }
    Value::Table(entries)
}

fn toml_inline_table(table: &toml_edit::InlineTable, fallback: &Range<usize>) -> Value {
    let mut entries = Vec::new();
    for (key, value) in table {
        let key_span = table
            .get_key_value(key)
            .and_then(|(k, _)| k.span())
            .unwrap_or_else(|| fallback.clone());
        entries.push(Entry {
            key: key.to_owned(),
            node: toml_value(value),
            key_span,
        });
    }
    Value::Table(entries)
}

fn toml_item(item: &toml_edit::Item, fallback: &Range<usize>) -> Node {
    match item {
        toml_edit::Item::Value(v) => toml_value(v),
        toml_edit::Item::Table(t) => {
            let span = t.span().unwrap_or_else(|| fallback.clone());
            Node {
                value: toml_table(t, &span),
                span,
            }
        }
        toml_edit::Item::ArrayOfTables(a) => {
            let items = a
                .iter()
                .map(|t| {
                    let span = t.span().unwrap_or_else(|| fallback.clone());
                    Node {
                        value: toml_table(t, &span),
                        span,
                    }
                })
                .collect();
            Node {
                span: fallback.clone(),
                value: Value::Array(items),
            }
        }
        toml_edit::Item::None => Node {
            span: fallback.clone(),
            value: Value::Null,
        },
    }
}

fn toml_value(v: &toml_edit::Value) -> Node {
    let span = v.span().unwrap_or(0..0);
    let value = match v {
        toml_edit::Value::String(s) => Value::Str(s.value().clone()),
        toml_edit::Value::Integer(i) => Value::Int(*i.value()),
        toml_edit::Value::Float(f) => Value::Float(*f.value()),
        toml_edit::Value::Boolean(b) => Value::Bool(*b.value()),
        toml_edit::Value::Datetime(d) => Value::Datetime(d.value().to_string()),
        toml_edit::Value::Array(a) => Value::Array(a.iter().map(toml_value).collect()),
        toml_edit::Value::InlineTable(t) => toml_inline_table(t, &span),
    };
    Node { span, value }
}

/// Parse a YAML document. JSON parses as YAML.
pub fn parse_yaml(text: &str) -> Result<Node, ParseError> {
    let docs = MarkedYaml::load_from_str(text).map_err(|e| {
        let at = char_to_byte(text, e.marker().index());
        ParseError {
            message: e.info().to_owned(),
            span: Some(at..at),
        }
    })?;
    let Some(first) = docs.first() else {
        return Ok(Node {
            span: 0..0,
            value: Value::Null,
        });
    };
    Ok(yaml_node(first, &ByteIndex::new(text)))
}

struct ByteIndex(Vec<usize>);

impl ByteIndex {
    fn new(text: &str) -> Self {
        let mut v: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
        v.push(text.len());
        Self(v)
    }

    fn at(&self, char_index: usize) -> usize {
        self.0
            .get(char_index)
            .copied()
            .unwrap_or_else(|| self.0.last().copied().unwrap_or(0))
    }

    fn range(&self, start: usize, end: usize) -> Range<usize> {
        self.at(start)..self.at(end)
    }
}

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(b, _)| b)
}

fn yaml_node(n: &MarkedYaml<'_>, idx: &ByteIndex) -> Node {
    let span = idx.range(n.span.start.index(), n.span.end.index());
    let value = match &n.data {
        YamlData::Value(Scalar::Boolean(b)) => Value::Bool(*b),
        YamlData::Value(Scalar::Integer(i)) => Value::Int(*i),
        YamlData::Value(Scalar::FloatingPoint(f)) => Value::Float(f.0),
        YamlData::Sequence(items) => {
            Value::Array(items.iter().map(|i| yaml_node(i, idx)).collect())
        }
        YamlData::Mapping(map) => Value::Table(
            map.iter()
                .map(|(k, v)| Entry {
                    key: yaml_key(k),
                    key_span: idx.range(k.span.start.index(), k.span.end.index()),
                    node: yaml_node(v, idx),
                })
                .collect(),
        ),
        YamlData::Value(Scalar::String(s)) | YamlData::Representation(s, ..) => {
            Value::Str(s.to_string())
        }
        _ => Value::Null,
    };
    Node { span, value }
}

fn yaml_key(k: &MarkedYaml<'_>) -> String {
    match &k.data {
        YamlData::Value(Scalar::String(s)) | YamlData::Representation(s, ..) => s.to_string(),
        YamlData::Value(Scalar::Integer(i)) => i.to_string(),
        YamlData::Value(Scalar::Boolean(b)) => b.to_string(),
        YamlData::Value(Scalar::FloatingPoint(f)) => f.0.to_string(),
        YamlData::Value(Scalar::Null) => "null".to_owned(),
        _ => "?".to_owned(),
    }
}

/// Maps byte offsets to one-based lines and columns, as `docs/contract/positions.md` defines.
///
/// A line ends at LF, CRLF or a CR that does not precede LF. Nothing else ends a line. A leading
/// byte order mark is not part of line 1, so offsets 0 and 3 are both line 1, column 1.
#[derive(Debug, Clone)]
pub struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
    /// For a line with a non-ASCII character: the code points before each byte of the line, so
    /// a column is one lookup. An ASCII line has none, because its column is its byte offset.
    chars_before: Vec<Option<Vec<u32>>>,
}

impl<'a> LineIndex<'a> {
    /// Index `text`.
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        let bom = if text.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        let bytes = text.as_bytes();
        let mut starts = vec![bom];
        for (i, &b) in bytes.iter().enumerate() {
            let ends_line = b == b'\n' || (b == b'\r' && bytes.get(i + 1) != Some(&b'\n'));
            if ends_line {
                starts.push(i + 1);
            }
        }
        let chars_before = starts
            .iter()
            .enumerate()
            .map(|(i, &start)| {
                let end = starts.get(i + 1).copied().unwrap_or(text.len());
                let line = text.get(start..end)?;
                if line.is_ascii() {
                    return None;
                }
                let mut table = Vec::with_capacity(line.len() + 1);
                let mut count = 0u32;
                for (at, _) in line.char_indices() {
                    while table.len() <= at {
                        table.push(count);
                    }
                    count += 1;
                }
                while table.len() <= line.len() {
                    table.push(count);
                }
                Some(table)
            })
            .collect();
        Self {
            text,
            starts,
            chars_before,
        }
    }

    /// One-based line and column of a byte offset. The column counts code points.
    #[must_use]
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.clamp(self.starts[0], self.text.len());
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let start = self.starts[line];
        let col = match &self.chars_before[line] {
            None => offset - start,
            Some(table) if self.text.is_char_boundary(offset) => table[offset - start] as usize,
            Some(_) => 0,
        };
        (line + 1, col + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_at<'a>(text: &'a str, r: &Range<usize>) -> &'a str {
        &text[r.clone()]
    }

    #[test]
    fn toml_keys_and_values_carry_spans() {
        let text = "mode = \"prose\"\n\n[thresholds]\nmax_errors = 3\n";
        let root = parse_toml(text).unwrap();
        let mode = root.entry("mode").unwrap();
        assert_eq!(text_at(text, &mode.key_span), "mode");
        assert_eq!(text_at(text, &mode.node.span), "\"prose\"");
        let errors = root.get("thresholds").unwrap().entry("max_errors").unwrap();
        assert_eq!(text_at(text, &errors.key_span), "max_errors");
        assert_eq!(text_at(text, &errors.node.span), "3");
    }

    #[test]
    fn toml_array_of_tables_and_inline_tables() {
        let text = "[[overrides]]\nfiles = [\"a\"]\nprofile = { name = \"strict\" }\n";
        let root = parse_toml(text).unwrap();
        let Value::Array(items) = &root.get("overrides").unwrap().value else {
            panic!("overrides is not an array");
        };
        assert_eq!(items.len(), 1);
        let name = items[0].get("profile").unwrap().entry("name").unwrap();
        assert_eq!(text_at(text, &name.key_span), "name");
        assert_eq!(text_at(text, &name.node.span), "\"strict\"");
    }

    #[test]
    fn yaml_spans_are_byte_offsets_after_multibyte_text() {
        let text = "# é — note\nlocale: en-GB\n";
        let root = parse_yaml(text).unwrap();
        let locale = root.entry("locale").unwrap();
        assert_eq!(text_at(text, &locale.key_span), "locale");
        assert_eq!(text_at(text, &locale.node.span), "en-GB");
    }

    #[test]
    fn json_round_trip_drops_spans() {
        let root = parse_toml("a = 1\nb = [\"x\"]\n").unwrap();
        assert_eq!(root.to_json(), serde_json::json!({"a": 1, "b": ["x"]}));
    }

    #[test]
    fn shift_moves_every_span() {
        let mut root = parse_yaml("a: b\n").unwrap();
        root.shift(10);
        let e = root.entry("a").unwrap();
        assert_eq!(e.key_span, 10..11);
        assert_eq!(e.node.span, 13..14);
    }

    #[test]
    fn line_col_counts_characters() {
        let text = "ab\néx = 1\n";
        let idx = LineIndex::new(text);
        assert_eq!(idx.line_col(0), (1, 1));
        assert_eq!(idx.line_col(3), (2, 1));
        assert_eq!(idx.line_col(text.find('x').unwrap()), (2, 2));
    }

    #[test]
    fn columns_on_one_very_long_line_take_linear_time() {
        use std::fmt::Write as _;
        // 100k keys on one line, led by a non-ASCII character so every column needs a count.
        let mut text = String::from("é = {");
        let mut offsets = Vec::new();
        for i in 0..100_000 {
            offsets.push(text.len());
            let _ = write!(text, "k{i} = 1, ");
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let owned = text.clone();
        let wanted = offsets.clone();
        std::thread::spawn(move || {
            let idx = LineIndex::new(&owned);
            let cols: Vec<usize> = wanted.iter().map(|o| idx.line_col(*o).1).collect();
            let _ = tx.send(cols);
        });
        let cols = rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("column computation is not linear");
        // The first key starts after `é = {`, which is five characters.
        assert_eq!(cols[0], 6);
        assert_eq!(cols[1] - cols[0], "k0 = 1, ".len());
        assert_eq!(
            cols[99_999],
            6 + text[offsets[0]..offsets[99_999]].chars().count()
        );
    }

    #[test]
    fn toml_syntax_error_reports_a_span() {
        let err = parse_toml("a = = 1\n").unwrap_err();
        assert!(err.span.is_some());
    }
}
