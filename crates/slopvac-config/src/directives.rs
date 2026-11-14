//! Inline directives: `slopvac: key=value` comments for documents.
//!
//! A directive sets the same fields as the front-matter block, so formats without front matter
//! (reStructuredText, plain text, code) can carry per-file settings. The same `[frontmatter]
//! allow` list decides which fields take effect.
//!
//! | Document | Form |
//! |---|---|
//! | Markdown, HTML | `<!-- slopvac: genre=how-to locale=en-GB -->` |
//! | reStructuredText | `.. slopvac: genre=how-to` |
//! | Plain text and code | `# slopvac: ...`, `// slopvac: ...`, `-- slopvac: ...`, `; slopvac: ...` or `/* slopvac: ... */` |
//!
//! A dotted key sets a nested field: `thresholds.min_score=60`, `rules.core:no-dash=off`.
//! `terms.allow` takes a comma-separated list.

use std::ops::Range;
use std::path::Path;

use crate::diag::{Code, Diagnostic};
use crate::flatten::{Origin, flatten};
use crate::frontmatter::{Policy, Read, denied};
use crate::schema::validate_directive;
use crate::source::Layer;
use crate::tree::{Entry, Node, Value};

/// One directive comment: where it is and the fields it sets.
#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    /// Byte range of the text after `slopvac:`.
    pub payload: Range<usize>,
    /// The fields as a table. Every span indexes the document.
    pub node: Node,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Html,
    Rst,
    Line,
}

fn style_of(file: &str) -> Style {
    match Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "markdown" | "mdx" | "html" | "htm") => Style::Html,
        Some("rst") => Style::Rst,
        _ => Style::Line,
    }
}

/// Find the directives in `doc`. `file` picks the comment style by its extension.
#[must_use]
pub fn find(doc: &str, file: &str) -> Vec<Directive> {
    let style = style_of(file);
    let mut out = Vec::new();
    let mut offset = 0;
    for line in doc.split_inclusive('\n') {
        if let Some(payload) = payload_of(line, style) {
            let start = offset + payload.start;
            let range = start..offset + payload.end;
            out.push(Directive {
                node: parse_pairs(doc, range.clone()),
                payload: range,
            });
        }
        offset += line.len();
    }
    out
}

/// The byte range of the text after `slopvac:` in a directive comment, if `line` is one.
fn payload_of(line: &str, style: Style) -> Option<Range<usize>> {
    let lead = line.len() - line.trim_start().len();
    let body = &line[lead..];
    let (opens, closes): (&[&str], &[&str]) = match style {
        Style::Html => (&["<!--"], &["-->"]),
        Style::Rst => (&[".."], &[]),
        Style::Line => (&["//", "#", "--", ";", "/*"], &["*/"]),
    };
    let open = opens.iter().find(|p| body.starts_with(**p))?;
    let after_open = lead + open.len();
    let rest = &line[after_open..];
    let skipped = rest.len() - rest.trim_start().len();
    let key_at = after_open + skipped;
    let payload_at = key_at + "slopvac:".len();
    if !line[key_at..].starts_with("slopvac:") {
        return None;
    }
    let mut end = line.trim_end().len();
    for close in closes {
        if line[..end].ends_with(close) {
            end -= close.len();
            end = line[..end].trim_end().len();
            break;
        }
    }
    if style == Style::Html && !line.trim_end().ends_with("-->") {
        return None;
    }
    Some(payload_at..end.max(payload_at))
}

/// Split `key=value` pairs at white space; a value may be in double quotes.
fn parse_pairs(doc: &str, payload: Range<usize>) -> Node {
    let text = &doc[payload.clone()];
    let mut entries: Vec<Entry> = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key_end = i;
        let key = &text[key_start..key_end];
        let key_span = payload.start + key_start..payload.start + key_end;
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            let (value, value_span, next) = read_value(text, i, payload.start);
            i = next;
            insert(&mut entries, key, key_span, value, value_span);
        } else {
            insert(
                &mut entries,
                key,
                key_span.clone(),
                Value::Bool(true),
                key_span,
            );
        }
    }
    Node {
        span: payload,
        value: Value::Table(entries),
    }
}

fn read_value(text: &str, from: usize, base: usize) -> (String, Range<usize>, usize) {
    let bytes = text.as_bytes();
    if bytes.get(from) == Some(&b'"') {
        let close = text[from + 1..]
            .find('"')
            .map_or(text.len(), |p| from + 1 + p);
        let end = (close + 1).min(text.len());
        return (
            text[from + 1..close].to_owned(),
            base + from..base + end,
            end,
        );
    }
    let mut i = from;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    (text[from..i].to_owned(), base + from..base + i, i)
}

fn typed(key: &str, raw: &str, span: &Range<usize>) -> Value {
    if key == "terms.allow" {
        return Value::Array(
            raw.split(',')
                .filter(|w| !w.is_empty())
                .map(|w| Node {
                    span: span.clone(),
                    value: Value::Str(w.to_owned()),
                })
                .collect(),
        );
    }
    if let Ok(i) = raw.parse::<i64>() {
        Value::Int(i)
    } else if let Ok(f) = raw.parse::<f64>() {
        Value::Float(f)
    } else if raw == "true" || raw == "false" {
        Value::Bool(raw == "true")
    } else {
        Value::Str(raw.to_owned())
    }
}

/// Add `dotted.key = value`, creating or extending the nested tables.
fn insert(
    entries: &mut Vec<Entry>,
    dotted: &str,
    key_span: Range<usize>,
    raw: impl Into<Rawish>,
    value_span: Range<usize>,
) {
    let raw = raw.into();
    let mut offset = key_span.start;
    let segments: Vec<&str> = dotted.split('.').collect();
    let mut cur = entries;
    for (n, seg) in segments.iter().enumerate() {
        let span = offset..offset + seg.len();
        offset += seg.len() + 1;
        let last = n + 1 == segments.len();
        let pos = cur.iter().position(|e| e.key == *seg);
        if last {
            let value = match &raw {
                Rawish::Str(s) => typed(dotted, s, &value_span),
                Rawish::Value(v) => v.clone(),
            };
            let entry = Entry {
                key: (*seg).to_owned(),
                key_span: span,
                node: Node {
                    span: value_span.clone(),
                    value,
                },
            };
            match pos {
                Some(p) => cur[p] = entry,
                None => cur.push(entry),
            }
            return;
        }
        let p = match pos {
            Some(p) if matches!(cur[p].node.value, Value::Table(_)) => p,
            _ => {
                let entry = Entry {
                    key: (*seg).to_owned(),
                    key_span: span.clone(),
                    node: Node {
                        span,
                        value: Value::Table(Vec::new()),
                    },
                };
                if let Some(p) = pos {
                    cur[p] = entry;
                    p
                } else {
                    cur.push(entry);
                    cur.len() - 1
                }
            }
        };
        let Value::Table(next) = &mut cur[p].node.value else {
            return;
        };
        cur = next;
    }
}

enum Rawish {
    Str(String),
    Value(Value),
}

impl From<String> for Rawish {
    fn from(s: String) -> Self {
        Rawish::Str(s)
    }
}

impl From<Value> for Rawish {
    fn from(v: Value) -> Self {
        Rawish::Value(v)
    }
}

/// Read every directive in `doc`. `file` is the path shown in provenance.
///
/// Directives apply in document order, so a later one wins. A field outside the policy's allow
/// list is an error, and then the document's directives contribute nothing.
#[must_use]
pub fn read(doc: &str, file: &str, policy: &Policy) -> Read {
    if !policy.enabled {
        return Read::default();
    }
    let directives = find(doc, file);
    let mut diagnostics = Vec::new();
    for d in &directives {
        diagnostics.extend(validate_directive(&d.node, doc));
        diagnostics.extend(denied(&d.node, policy, "directive"));
        if let Value::Table(entries) = &d.node.value
            && entries.is_empty()
        {
            diagnostics.push(Diagnostic {
                code: Code::MissingKey,
                message: "the directive sets nothing".into(),
                label: "write `key=value` after `slopvac:`".into(),
                span: d.payload.clone(),
                help: None,
                path: vec!["directive".into()],
            });
        }
    }
    if !diagnostics.is_empty() {
        return Read {
            settings: Vec::new(),
            diagnostics,
        };
    }
    let settings = directives
        .iter()
        .flat_map(|d| {
            flatten(
                &d.node,
                Origin {
                    layer: Layer::Directive,
                    file,
                    text: doc,
                },
                &[],
            )
        })
        .collect();
    Read {
        settings,
        diagnostics,
    }
}
