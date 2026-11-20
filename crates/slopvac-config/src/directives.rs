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

use crate::comments::{Family, comments, family_of, family_of_name};
use crate::diag::{Code, Diagnostic};
use crate::flatten::{Origin, flatten};
use crate::frontmatter::{Policy, Read, denied};
use crate::messages::ConfigPhrase;
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
    /// Source code whose comment syntax is known, so strings can be told from comments.
    Code(Family),
    /// Text with no known comment syntax: a marker at the start of a line counts.
    Plain,
    Html,
    Rst,
}

fn style_of(file: &str) -> Style {
    let path = Path::new(file);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("md" | "markdown" | "mdx" | "html" | "htm") => Style::Html,
        Some("rst") => Style::Rst,
        Some(other) => family_of(other).map_or(Style::Plain, Style::Code),
        None => path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(family_of_name)
            .map_or(Style::Plain, Style::Code),
    }
}

/// Find the directives in `doc`. `file` picks the comment style by its extension.
///
/// Only real comments count. Lines inside a Markdown or HTML code block, a reStructuredText
/// literal block, or a multi-line string in code are examples or data, not directives.
#[must_use]
pub fn find(doc: &str, file: &str) -> Vec<Directive> {
    let style = style_of(file);
    if let Style::Code(family) = style {
        return find_in_code(doc, family);
    }
    let mut literal = Literals::new(style);
    let mut out = Vec::new();
    let mut offset = 0;
    for line in doc.split_inclusive('\n') {
        let in_literal = literal.step(line);
        if !in_literal && let Some(payload) = payload_of(line, style) {
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

/// Directives in the comments of source code, found with a scanner that knows the language's
/// strings. A comment counts when it is the first thing on its line.
fn find_in_code(doc: &str, family: Family) -> Vec<Directive> {
    let mut out = Vec::new();
    for c in comments(doc, family) {
        let line_start = doc.as_bytes()[..c.marker]
            .iter()
            .rposition(|&b| b == b'\n' || b == b'\r')
            .map_or(0, |p| p + 1);
        if !doc[line_start..c.marker].trim().is_empty() {
            continue;
        }
        let body = &doc[c.body.clone()];
        let text = body.trim_start();
        let Some(rest) = text.strip_prefix("slopvac:") else {
            continue;
        };
        let start = c.body.start + (body.len() - rest.len());
        let end = (c.body.start + body.trim_end().len()).max(start);
        out.push(Directive {
            node: parse_pairs(doc, start..end),
            payload: start..end,
        });
    }
    out
}

/// Tracks, line by line, whether the text is inside a code block or string.
struct Literals {
    style: Style,
    fence: Option<(char, usize)>,
    raw_tag: Option<&'static str>,
    rst_pending: bool,
    rst_active: bool,
}

const RAW_TAGS: &[&str] = &["pre", "script", "style", "code", "textarea"];
const RST_LITERAL_DIRECTIVES: &[&str] = &[
    ".. code-block::",
    ".. code::",
    ".. sourcecode::",
    ".. parsed-literal::",
];

impl Literals {
    fn new(style: Style) -> Self {
        Self {
            style,
            fence: None,
            raw_tag: None,
            rst_pending: false,
            rst_active: false,
        }
    }

    /// Feed the next line; true when that line is inside a literal context.
    fn step(&mut self, line: &str) -> bool {
        match self.style {
            Style::Html => self.step_markup(line),
            Style::Rst => self.step_rst(line),
            Style::Code(_) | Style::Plain => false,
        }
    }

    fn step_markup(&mut self, line: &str) -> bool {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let body = line.trim_start_matches(' ');
        if let Some((ch, len)) = self.fence {
            let run = body.chars().take_while(|c| *c == ch).count();
            if run >= len && body[run..].trim().is_empty() {
                self.fence = None;
            }
            return true;
        }
        if indent <= 3
            && let Some(ch) = body.chars().next().filter(|c| *c == '`' || *c == '~')
        {
            let run = body.chars().take_while(|c| *c == ch).count();
            if run >= 3 && !(ch == '`' && body[run..].contains('`')) {
                self.fence = Some((ch, run));
                return true;
            }
        }
        if let Some(tag) = self.raw_tag {
            if line.to_ascii_lowercase().contains(&format!("</{tag}")) {
                self.raw_tag = None;
            }
            return true;
        }
        let lower = line.to_ascii_lowercase();
        for tag in RAW_TAGS {
            if let Some(at) = lower.find(&format!("<{tag}"))
                && !lower[at..].contains(&format!("</{tag}"))
            {
                self.raw_tag = Some(tag);
                return true;
            }
        }
        indent >= 4 || line.starts_with('\t')
    }

    fn step_rst(&mut self, line: &str) -> bool {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let blank = line.trim().is_empty();
        if self.rst_active {
            if blank || indented {
                return true;
            }
            self.rst_active = false;
        }
        if self.rst_pending {
            if blank {
                return false;
            }
            self.rst_pending = false;
            if indented {
                self.rst_active = true;
                return true;
            }
        }
        let trimmed = line.trim_end();
        let lower = trimmed.to_ascii_lowercase();
        if RST_LITERAL_DIRECTIVES.iter().any(|d| lower.starts_with(d))
            || (trimmed.ends_with("::") && !trimmed.starts_with(".. "))
        {
            self.rst_pending = true;
        }
        false
    }
}

/// The byte range of the text after `slopvac:` in a directive comment, if `line` is one.
fn payload_of(line: &str, style: Style) -> Option<Range<usize>> {
    let lead = line.len() - line.trim_start().len();
    let body = &line[lead..];
    let (opens, closes): (&[&str], &[&str]) = match style {
        Style::Html => (&["<!--"], &["-->"]),
        Style::Rst => (&[".."], &[]),
        Style::Plain | Style::Code(_) => (&["//", "#", "--", ";", "/*"], &["*/"]),
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
                message: ConfigPhrase::DirectiveEmpty.text(),
                label: ConfigPhrase::LabelWriteKeyValue.text(),
                span: d.payload.clone(),
                help: None,
                path: vec!["directive".into()],
                data: None,
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
