//! Front matter: find the block, read the `slopvac:` mapping, and apply the allow policy.
//!
//! Detection and parsing come from the `frontmatter-spans` crate; this module adds the allow
//! policy and the `slopvac:` mapping.

use crate::sub;
pub use frontmatter_spans::{Block, Format};
use frontmatter_spans::{FrontMatter, Limits};

use crate::diag::{Code, Diagnostic};
use crate::flatten::{Origin, flatten};
use crate::load::LoadedConfig;
use crate::schema::validate_frontmatter;
use crate::source::{Layer, Setting};
use crate::tree::{Entry, Node, Value};

/// Lines with their start offsets. A line ends at LF, CRLF or a CR that does not precede LF, as
/// `docs/contract/positions.md` defines, and keeps its terminator.
fn lines_with_offsets(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while let Some(&current) = bytes.get(i) {
        match current {
            b'\n' => i += 1,
            b'\r' => {
                i += if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                }
            }
            _ => {
                i += 1;
                continue;
            }
        }
        lines.push((start, sub(text, start..i)));
        start = i;
    }
    if start < bytes.len() {
        lines.push((start, sub(text, start..)));
    }
    lines.into_iter()
}

/// Find the front-matter block at the start of `doc`.
///
/// A fence line closes or opens a block when it equals the fence once all trailing Unicode
/// whitespace is trimmed, as in 2.x. `frontmatter_spans::detect` trims only spaces and tabs and
/// no contract text says which is right, so the old rule stays here. A JSON object goes to the
/// crate unchanged.
#[must_use]
pub fn extract(doc: &str) -> Option<Block> {
    let start = if doc.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let rest = sub(doc, start..);
    let (format, fence) = if rest.starts_with('{') {
        return frontmatter_spans::detect(doc);
    } else if rest.starts_with("---") {
        (Format::Yaml, "---")
    } else if rest.starts_with("+++") {
        (Format::Toml, "+++")
    } else {
        return None;
    };
    let mut lines = lines_with_offsets(rest);
    let (_, first) = lines.next()?;
    if first.trim_end() != fence {
        return None;
    }
    let content_start = start + first.len();
    for (offset, line) in lines {
        let t = line.trim_end();
        if t == fence || (format == Format::Yaml && t == "...") {
            let line_start = start + offset;
            let terminator = line.len() - line.trim_end_matches(['\r', '\n']).len();
            return Some(Block {
                format,
                outer: start..line_start + line.len() - terminator,
                content: content_start..line_start,
                body_start: line_start + line.len(),
            });
        }
    }
    None
}

/// Parse a block found in `doc`. Every span in the tree indexes `doc`, not the block.
///
/// A repeated key in a table is an error in every format. YAML and TOML refuse it in their
/// parsers; JSON keeps every member, so the check runs here, with the message YAML gives.
pub fn parse_block(doc: &str, block: &Block) -> Result<Node, crate::tree::ParseError> {
    let node = FrontMatter::parse_block_with(doc, block.clone(), &Limits::default())
        .map(|fm| convert(&fm.root))
        .map_err(|e| crate::tree::ParseError {
            message: e.message,
            span: e.span.map(|s| s.range),
        })?;
    if let Some(entry) = first_duplicate(&node) {
        return Err(crate::tree::ParseError {
            message: format!("duplicate key `{}`", entry.key),
            span: Some(entry.key_span.clone()),
        });
    }
    Ok(node)
}

/// The first entry, in document order, whose key an earlier entry of the same table has.
fn first_duplicate(node: &Node) -> Option<&Entry> {
    match &node.value {
        Value::Table(entries) => {
            // A set per table keeps the check linear in the number of keys.
            let mut seen = std::collections::HashSet::with_capacity(entries.len());
            entries.iter().find_map(|e| {
                if seen.insert(e.key.as_str()) {
                    first_duplicate(&e.node)
                } else {
                    Some(e)
                }
            })
        }
        Value::Array(items) => items.iter().find_map(first_duplicate),
        Value::Null
        | Value::Bool(_)
        | Value::Int(_)
        | Value::Float(_)
        | Value::Str(_)
        | Value::Datetime(_) => None,
    }
}

/// The crate's tree as this crate's tree, so validation and flattening do not depend on it.
fn convert(node: &frontmatter_spans::Node) -> Node {
    use frontmatter_spans::Value as V;
    let value = match &node.value {
        V::Null => Value::Null,
        V::Bool(b) => Value::Bool(*b),
        V::Integer(i) => Value::Int(*i),
        V::Float(f) => Value::Float(*f),
        V::String(s) => Value::Str(s.clone()),
        V::Datetime(s) => Value::Datetime(s.clone()),
        V::Array(items) => Value::Array(items.iter().map(convert).collect()),
        V::Table(entries) => Value::Table(
            entries
                .iter()
                .map(|e| Entry {
                    key: e.key.clone(),
                    key_span: e.key_span.range.clone(),
                    node: convert(&e.value),
                })
                .collect(),
        ),
    };
    Node {
        span: node.span.range.clone(),
        value,
    }
}

/// The project's `[frontmatter]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// When false, `slopvac:` is ordinary data and is not read.
    pub enabled: bool,
    /// The fields of the block that take effect.
    pub allow: Vec<String>,
}

impl Policy {
    /// The core defaults, for a document with no config.
    #[must_use]
    pub fn core() -> Self {
        Self {
            enabled: true,
            allow: ["genre", "locale", "terms"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        }
    }

    /// The policy a loaded config sets, with the core defaults under it.
    #[must_use]
    pub fn from_config(config: &LoadedConfig) -> Self {
        let enabled = config
            .project_value(&["frontmatter", "enabled"])
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let allow = config
            .project_value(&["frontmatter", "allow"])
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        Self { enabled, allow }
    }
}

/// What a document's front matter contributes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Read {
    /// Settings for layer `front-matter`. Empty when any diagnostic is present.
    pub settings: Vec<Setting>,
    /// Everything wrong with the block, in document order.
    pub diagnostics: Vec<Diagnostic>,
}

const SETTING_FIELDS: &[&str] = &[
    "genre",
    "profile",
    "locale",
    "thresholds",
    "categories",
    "rules",
    "terms",
];

/// Read the `slopvac:` block of `doc`. `file` is the path shown in provenance.
///
/// A document with no front matter, or none that holds a `slopvac` key, contributes nothing.
/// A field outside the policy's allow list is an error every time, and then the block
/// contributes nothing: a half-applied block would be worse than none.
#[must_use]
pub fn read(doc: &str, file: &str, policy: &Policy) -> Read {
    if !policy.enabled {
        return Read::default();
    }
    let Some(block) = extract(doc) else {
        return Read::default();
    };
    let raw = sub(doc, block.content.clone());
    let (format, content_start) = (block.format, block.content.start);
    // Cheap candidate check: the key `slopvac` can be spelled another way only through an escape,
    // so text with neither the word nor a backslash cannot hold the block and is never parsed.
    if !raw.contains("slopvac") && !raw.contains('\\') {
        return Read::default();
    }
    let root = match parse_block(doc, &block) {
        Ok(n) => n,
        // Ownership: a block that does not parse is ours only if a top-level key decodes to
        // `slopvac`. The word inside a value or a nested key says nothing.
        Err(_) if !has_slopvac_key(raw, format) => return Read::default(),
        Err(e) => {
            let at = e.span.map_or(content_start, |s| s.start);
            return Read {
                settings: Vec::new(),
                diagnostics: vec![Diagnostic {
                    code: Code::Parse,
                    message: format!("the front matter does not parse: {}", e.message),
                    label: "the parser stopped here".into(),
                    span: at..at,
                    help: None,
                    path: Vec::new(),
                }],
            };
        }
    };
    if root.get("slopvac").is_none() {
        return Read::default();
    }
    let mut diagnostics = validate_frontmatter(&root, doc);
    if let Some(slopvac) = root.get("slopvac") {
        diagnostics.extend(denied(slopvac, policy, "slopvac"));
    }
    diagnostics.sort_by_key(|d| d.span.start);
    if !diagnostics.is_empty() {
        return Read {
            settings: Vec::new(),
            diagnostics,
        };
    }
    let settings = root.get("slopvac").map_or_else(Vec::new, |block| {
        flatten(
            block,
            Origin {
                layer: Layer::FrontMatter,
                file,
                text: doc,
            },
            &["v"],
        )
    });
    Read {
        settings,
        diagnostics,
    }
}

/// The fields of `block` that `policy` does not allow. `root` is the first key-path segment
/// (`slopvac` or `directive`), and it also names the source in the message.
pub(crate) fn denied(block: &Node, policy: &Policy, root: &str) -> Vec<Diagnostic> {
    let what = if root == "directive" {
        "an inline directive"
    } else {
        "front matter"
    };
    let Value::Table(entries) = &block.value else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|e| SETTING_FIELDS.contains(&e.key.as_str()))
        .filter(|e| !policy.allow.contains(&e.key))
        .map(|e| Diagnostic {
            code: Code::FrontmatterFieldDenied,
            message: format!("the project does not let {what} set `{}`", e.key),
            label: "not in `[frontmatter] allow`".into(),
            span: e.key_span.clone(),
            help: Some(format!(
                "to allow `{}`, add it to `[frontmatter] allow` in slopvac.toml",
                e.key
            )),
            path: vec![root.to_owned(), e.key.clone()],
        })
        .collect()
}

/// Whether the top-level keys of `raw` include `slopvac` once escapes are decoded.
///
/// This runs only when the block does not parse, so the parser cannot say which keys it holds.
/// It reads the text by format and looks at top-level keys only: a quoted value or a nested
/// key that decodes to `slopvac` does not count. It is a best effort on broken text.
fn has_slopvac_key(raw: &str, format: Format) -> bool {
    match format {
        Format::Yaml => lines_with_offsets(raw).any(|(_, line)| {
            !line.starts_with([' ', '\t', '#', '-'])
                && leading_key(line).is_some_and(|k| k == "slopvac")
        }),
        Format::Json => json_top_level_keys(raw).iter().any(|k| k == "slopvac"),
        Format::Toml => toml_top_level_names(raw).iter().any(|k| k == "slopvac"),
    }
}

/// The key a YAML line starts with: a quoted string followed by a colon, or a bare word.
fn leading_key(line: &str) -> Option<String> {
    let (key, rest) = take_key(line)?;
    rest.trim_start().starts_with(':').then_some(key)
}

/// A bare or quoted key at the start of `s`, and the text after it.
fn take_key(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start_matches([' ', '\t']);
    if let Some(body) = s.strip_prefix('"') {
        let bytes = body.as_bytes();
        let mut j = 0;
        while bytes.get(j).is_some_and(|&c| c != b'"') {
            j += if bytes.get(j) == Some(&b'\\') { 2 } else { 1 };
        }
        if j >= bytes.len() {
            return None;
        }
        return Some((decode_escapes(sub(body, ..j)), sub(body, j + 1..)));
    }
    if let Some(body) = s.strip_prefix('\'') {
        let end = body.find('\'')?;
        return Some((sub(body, ..end).to_owned(), sub(body, end + 1..)));
    }
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(s.len());
    (end > 0).then(|| (sub(s, ..end).to_owned(), sub(s, end..)))
}

/// Keys of the outermost JSON object, found by tracking nesting and strings.
fn json_top_level_keys(raw: &str) -> Vec<String> {
    let bytes = raw.as_bytes();
    let mut keys = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(&current) = bytes.get(i) {
        match current {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            b'"' => {
                let start = i + 1;
                let mut j = start;
                while bytes.get(j).is_some_and(|&c| c != b'"') {
                    j += if bytes.get(j) == Some(&b'\\') { 2 } else { 1 };
                }
                let end = j.min(bytes.len());
                let after = raw.get(end + 1..).unwrap_or("").trim_start();
                if depth == 1 && after.starts_with(':') {
                    keys.push(decode_escapes(sub(raw, start..end)));
                }
                i = end;
            }
            _ => {}
        }
        i += 1;
    }
    keys
}

/// The first key of every TOML table header, and of each root key before the first header.
fn toml_top_level_names(raw: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_root = true;
    for (_, line) in lines_with_offsets(raw) {
        let t = line.trim_start();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if let Some(header) = t.strip_prefix('[') {
            in_root = false;
            let header = header.strip_prefix('[').unwrap_or(header);
            if let Some((name, _)) = take_key(header) {
                names.push(name);
            }
        } else if in_root && let Some((name, _)) = take_key(t) {
            names.push(name);
        }
    }
    names
}

/// Decode the escapes that YAML, JSON and TOML double-quoted strings share.
fn decode_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(e) = chars.next() else { break };
        let digits = match e {
            'u' => 4,
            'U' => 8,
            'x' => 2,
            _ => {
                out.push(match e {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    other => other,
                });
                continue;
            }
        };
        let hex: String = chars.by_ref().take(digits).collect();
        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
            out.push(ch);
        }
    }
    out
}
