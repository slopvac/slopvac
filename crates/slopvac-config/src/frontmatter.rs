//! Front matter: find the block, read the `slopvac:` mapping, and apply the allow policy.
//!
//! The extraction half is a local stand-in for the `slopvac-frontmatter` crate.
// STUB(slopvac-8l4.4.8): `extract` and `parse_block` fake the publishable front-matter crate
// (slopvac-8l4.3.4): YAML (`---`), TOML (`+++`) and JSON detection with document-absolute
// spans. Swap them for the crate; `read` and `Policy` stay.

use std::ops::Range;

use crate::diag::{Code, Diagnostic};
use crate::flatten::{Origin, flatten};
use crate::load::LoadedConfig;
use crate::schema::validate_frontmatter;
use crate::source::{Layer, Setting};
use crate::tree::{Node, Value, parse_toml, parse_yaml};

/// The syntax of a front-matter block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Between `---` lines.
    Yaml,
    /// Between `+++` lines.
    Toml,
    /// A JSON object at the start of the document.
    Json,
}

/// A front-matter block located in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The syntax.
    pub format: Format,
    /// Byte range of the block content, without the delimiter lines.
    pub content: Range<usize>,
}

/// Find the front-matter block at the start of `doc`.
#[must_use]
pub fn extract(doc: &str) -> Option<Block> {
    let doc_start = doc
        .strip_prefix('\u{feff}')
        .map_or(0, |_| '\u{feff}'.len_utf8());
    let rest = &doc[doc_start..];
    if rest.starts_with('{') {
        return json_block(rest, doc_start);
    }
    let (format, fence) = if rest.starts_with("---") {
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
    let content_start = doc_start + first.len();
    for (offset, line) in lines {
        let t = line.trim_end();
        if t == fence || (format == Format::Yaml && t == "...") {
            return Some(Block {
                format,
                content: content_start..doc_start + offset,
            });
        }
    }
    None
}

fn lines_with_offsets(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split_inclusive('\n').scan(0, |pos, line| {
        let at = *pos;
        *pos += line.len();
        Some((at, line))
    })
}

fn json_block(rest: &str, doc_start: usize) -> Option<Block> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(Block {
                        format: Format::Json,
                        content: doc_start..doc_start + i + 1,
                    });
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse a block. Every span in the tree indexes `doc`, not the block.
pub fn parse_block(doc: &str, block: &Block) -> Result<Node, crate::tree::ParseError> {
    let text = &doc[block.content.clone()];
    let mut node = match block.format {
        Format::Toml => parse_toml(text)?,
        Format::Yaml | Format::Json => parse_yaml(text)?,
    };
    node.shift(block.content.start);
    Ok(node)
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
    let raw = &doc[block.content.clone()];
    // The key `slopvac` can be spelled another way only through an escape, so text with neither
    // the word nor a backslash cannot hold the block. Otherwise the decoded keys decide.
    let names_slopvac = raw.contains("slopvac");
    if !names_slopvac && !raw.contains('\\') {
        return Read::default();
    }
    let root = match parse_block(doc, &block) {
        Ok(n) => n,
        Err(_) if !names_slopvac => return Read::default(),
        Err(e) => {
            let at = e
                .span
                .map_or(block.content.start, |s| s.start + block.content.start);
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
        diagnostics.extend(denied(slopvac, policy));
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

fn denied(block: &Node, policy: &Policy) -> Vec<Diagnostic> {
    let Value::Table(entries) = &block.value else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|e| SETTING_FIELDS.contains(&e.key.as_str()))
        .filter(|e| !policy.allow.contains(&e.key))
        .map(|e| Diagnostic {
            code: Code::FrontmatterFieldDenied,
            message: format!("the project does not let front matter set `{}`", e.key),
            label: "not in `[frontmatter] allow`".into(),
            span: e.key_span.clone(),
            help: Some(format!(
                "to allow `{}`, add it to `[frontmatter] allow` in slopvac.toml",
                e.key
            )),
            path: vec!["slopvac".into(), e.key.clone()],
        })
        .collect()
}
