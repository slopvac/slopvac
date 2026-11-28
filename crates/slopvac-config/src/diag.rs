//! Diagnostics: coded errors that carry a span, rendered with `annotate-snippets`.

use std::fmt;
use std::ops::Range;

use annotate_snippets::{AnnotationKind, Group, Level, Origin, Renderer, Snippet};

use slopvac_messages::Catalog;

use crate::messages::{ConfigMessage, ConfigPhrase};
use crate::tree::LineIndex;

/// Stable identifier of a diagnostic, printed in brackets and matched by tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    /// A file could not be read.
    Io,
    /// A config file is larger than [`crate::MAX_CONFIG_BYTES`].
    TooLarge,
    /// A style file is larger than the style cap.
    StyleTooLarge,
    /// A document is larger than [`crate::MAX_DOCUMENT_BYTES`].
    DocumentTooLarge,
    /// The file is not valid TOML or YAML.
    Parse,
    /// A key the schema does not define.
    UnknownKey,
    /// A value of the wrong type.
    InvalidType,
    /// A value of the right type that the schema refuses.
    InvalidValue,
    /// A required key is absent.
    MissingKey,
    /// A 2.x config: no `schema_version`, or version 2.
    LegacyVersion,
    /// A `schema_version` newer than this build supports.
    UnsupportedVersion,
    /// An `extends` chain that loops back on itself.
    ExtendsCycle,
    /// A config that passes a budget: `extends` depth, files applied or bytes read, or blocks in
    /// one file. The diagnostic's `data` names which.
    BudgetExceeded,
    /// An `extends` entry that names no readable config.
    ExtendsMissing,
    /// Two `[[overrides]]` or `[[genres]]` blocks with the same `files` set.
    DuplicateScope,
    /// A front-matter field that `[frontmatter] allow` does not list.
    FrontmatterFieldDenied,
}

impl Code {
    /// The identifier text, such as `config-unknown-key`. The message catalog owns it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        let text = String::new;
        match self {
            Code::Io => ConfigMessage::Io {
                path: text(),
                reason: text(),
            }
            .code(),
            Code::TooLarge => ConfigMessage::TooLarge {
                path: text(),
                cap: 0,
            }
            .code(),
            Code::StyleTooLarge => ConfigMessage::StyleTooLarge {
                path: text(),
                cap: 0,
            }
            .code(),
            Code::DocumentTooLarge => ConfigMessage::DocumentTooLarge {
                path: text(),
                cap: 0,
            }
            .code(),
            Code::Parse => ConfigMessage::Parse { message: text() }.code(),
            Code::UnknownKey => ConfigMessage::UnknownKey { message: text() }.code(),
            Code::InvalidType => ConfigMessage::InvalidType { message: text() }.code(),
            Code::InvalidValue => ConfigMessage::InvalidValue { message: text() }.code(),
            Code::MissingKey => ConfigMessage::MissingKey { message: text() }.code(),
            Code::LegacyVersion => ConfigMessage::LegacyVersion { message: text() }.code(),
            Code::UnsupportedVersion => {
                ConfigMessage::UnsupportedVersion { message: text() }.code()
            }
            Code::ExtendsCycle => ConfigMessage::ExtendsCycle { message: text() }.code(),
            Code::BudgetExceeded => ConfigMessage::BudgetExceeded { message: text() }.code(),
            Code::ExtendsMissing => ConfigMessage::ExtendsMissing { message: text() }.code(),
            Code::DuplicateScope => ConfigMessage::DuplicateScope { message: text() }.code(),
            Code::FrontmatterFieldDenied => {
                ConfigMessage::FrontmatterFieldDenied { message: text() }.code()
            }
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which `extends` budget a run passed. The list can grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigBudget {
    /// Nesting depth, in levels: [`crate::MAX_EXTENDS_DEPTH`].
    ExtendsDepth,
    /// Files applied, in files: [`crate::MAX_EXTENDS_FILES`].
    ExtendsFiles,
    /// Bytes read, in bytes: [`crate::MAX_EXTENDS_BYTES`].
    ExtendsBytes,
    /// Bytes of config text applied, counting each application, in bytes:
    /// [`crate::MAX_EXTENDS_APPLIED_BYTES`].
    ExtendsAppliedBytes,
    /// `[[genres]]` and `[[overrides]]` blocks in one file, in blocks:
    /// [`crate::MAX_FILE_BLOCKS`].
    FileBlocks,
    /// `[[index.extra]]` entries in one file, in indexes: [`crate::MAX_INDEX_EXTRA`].
    IndexExtra,
}

impl ConfigBudget {
    /// The `data.budget` text, such as `extends-depth`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExtendsDepth => "extends-depth",
            Self::ExtendsFiles => "extends-files",
            Self::ExtendsBytes => "extends-bytes",
            Self::ExtendsAppliedBytes => "extends-applied-bytes",
            Self::FileBlocks => "file-blocks",
            Self::IndexExtra => "index-extra",
        }
    }
}

/// The machine-readable detail of a `config-budget-exceeded` diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetData {
    /// The budget that was passed.
    pub budget: ConfigBudget,
    /// The limit, in the unit of `budget`.
    pub limit: u64,
    /// The config file at which the limit tripped, relative to the root.
    pub path: String,
}

/// One problem, anchored to a byte range of one source text.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Diagnostic {
    /// Stable identifier.
    pub code: Code,
    /// One-line statement of the problem.
    pub message: String,
    /// Short text shown under the underlined span.
    pub label: String,
    /// Byte range the label points at.
    pub span: Range<usize>,
    /// What to do about it.
    pub help: Option<String>,
    /// Key path from the document root to the offending key or value.
    pub path: Vec<String>,
    /// Machine-readable detail, set only for a budget diagnostic.
    pub data: Option<Box<BudgetData>>,
}

impl Diagnostic {
    /// The path as a JSON pointer, `""` for the document root.
    #[must_use]
    pub fn pointer(&self) -> String {
        self.path.iter().fold(String::new(), |mut acc, seg| {
            acc.push('/');
            acc.push_str(&seg.replace('~', "~0").replace('/', "~1"));
            acc
        })
    }

    /// A fixed message for output that must not carry a value of the document. The message of
    /// the diagnostic itself can quote the value it refused (policies 1.7), so a caller that
    /// shows only the code and the span shows this text.
    #[must_use]
    pub fn redacted_message(&self) -> String {
        ConfigPhrase::LabelNotValidHere.text()
    }

    /// One-based line and column of the span start in `text`.
    #[must_use]
    pub fn location(&self, text: &str) -> (usize, usize) {
        self.location_in(&LineIndex::new(text))
    }

    /// One-based line and column of the span start, from a line index of the text built once
    /// for all the diagnostics of a document.
    #[must_use]
    pub fn location_in(&self, lines: &LineIndex<'_>) -> (usize, usize) {
        lines.line_col(self.span.start)
    }
}

/// A named source text.
#[derive(Debug, Clone, Copy)]
pub struct SourceFile<'a> {
    /// The path shown in the rendered output.
    pub name: &'a str,
    /// The full text the spans index into.
    pub text: &'a str,
}

/// Render every diagnostic as plain text, separated by blank lines.
///
/// The value of every `index.extra[*].source` is masked in the quoted text. A file whose name
/// ends in `.toml` that does not parse gets no excerpt at all, only its location, because the
/// masking needs a parsed document.
#[must_use]
pub fn render(source: SourceFile<'_>, diagnostics: &[Diagnostic]) -> String {
    let is_toml = std::path::Path::new(source.name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
    render_as(source, diagnostics, is_toml)
}

/// Render the diagnostics of a TOML config, which gets no excerpt when it does not parse.
pub(crate) fn render_config(source: SourceFile<'_>, diagnostics: &[Diagnostic]) -> String {
    render_as(source, diagnostics, true)
}

fn render_as(source: SourceFile<'_>, diagnostics: &[Diagnostic], toml_config: bool) -> String {
    let renderer = Renderer::plain();
    match redact_index_sources(source.text) {
        Some(masked) => {
            let source = SourceFile {
                name: source.name,
                text: &masked,
            };
            diagnostics
                .iter()
                .map(|d| render_one(&renderer, source, d))
                .collect::<Vec<_>>()
                .join("\n")
        }
        None if toml_config => diagnostics
            .iter()
            .map(|d| render_located(&renderer, source, d))
            .collect::<Vec<_>>()
            .join("\n"),
        None => diagnostics
            .iter()
            .map(|d| render_one(&renderer, source, d))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// One diagnostic with its file, line and column and no excerpt of the text.
fn render_located(renderer: &Renderer, source: SourceFile<'_>, d: &Diagnostic) -> String {
    let (line, column) = d.location(source.text);
    let mut report = vec![
        Level::ERROR
            .primary_title(d.message.clone())
            .id(d.code.as_str())
            .element(Origin::path(source.name).line(line).char_column(column)),
    ];
    if let Some(help) = &d.help {
        report.push(Group::with_title(Level::HELP.secondary_title(help.clone())));
    }
    renderer.render(&report)
}

/// The text with the value of every `index.extra[*].source` masked, because a source may carry a
/// credential and the snippet quotes the whole line. `None` when the text is not TOML.
///
/// The mask covers the span of the value under `source` of each entry, whatever the spelling of
/// the key and whatever the type of the value. It keeps the quotes of a string, line breaks and
/// byte length, so every span, line and column stays where it was. Each other byte becomes `*`.
fn redact_index_sources(text: &str) -> Option<std::borrow::Cow<'_, str>> {
    use crate::tree::{Node, Value, parse_toml};

    let root = parse_toml(text).ok()?;
    let mut nodes: Vec<&Node> = Vec::new();
    if let Some(extra) = root.get("index").and_then(|index| index.get("extra")) {
        match &extra.value {
            Value::Array(items) => nodes.extend(items.iter().filter_map(|item| item.get("source"))),
            Value::Table(_) => nodes.extend(extra.get("source")),
            _ => {}
        }
    }
    let mut spans: Vec<(Range<usize>, bool)> = nodes
        .into_iter()
        .map(|node| (node.span.clone(), matches!(node.value, Value::Str(_))))
        .filter(|(span, _)| {
            span.end <= text.len()
                && text.is_char_boundary(span.start)
                && text.is_char_boundary(span.end)
        })
        .collect();
    if spans.is_empty() {
        return Some(std::borrow::Cow::Borrowed(text));
    }
    spans.sort_by_key(|(s, _)| s.start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (span, is_string) in spans {
        if span.start < at {
            continue;
        }
        out.push_str(&text[at..span.start]);
        let literal = &text[span.clone()];
        if is_string {
            out.push_str(&mask_literal(literal));
        } else {
            out.extend(literal.chars().map(|c| star(c, true)));
        }
        at = span.end;
    }
    out.push_str(&text[at..]);
    Some(std::borrow::Cow::Owned(out))
}

/// A TOML string literal with its quotes kept and the rest replaced by `*`, byte for byte.
fn mask_literal(literal: &str) -> String {
    let quote = literal.chars().next().filter(|c| matches!(c, '"' | '\''));
    let Some(q) = quote else {
        return literal.chars().map(|c| star(c, false)).collect();
    };
    let width = if literal.starts_with(&q.to_string().repeat(3)) && literal.len() >= 6 {
        3
    } else {
        1
    };
    let inner_end = literal.len().saturating_sub(width).max(width);
    let (head, rest) = literal.split_at(width);
    let (inner, tail) = rest.split_at(inner_end - width);
    let masked: String = inner.chars().map(|c| star(c, true)).collect();
    format!("{head}{masked}{tail}")
}

/// `*` repeated for the bytes of `c`, so a masked line keeps its byte offsets. Line breaks stay
/// when `keep_breaks` is set.
fn star(c: char, keep_breaks: bool) -> String {
    if keep_breaks && matches!(c, '\n' | '\r') {
        c.to_string()
    } else {
        "*".repeat(c.len_utf8())
    }
}

/// The text as `annotate-snippets` should see it, and how far its offsets moved.
///
/// The renderer splits lines at LF and CRLF only and counts a byte order mark in column 1. The
/// position contract also ends a line at a lone CR and keeps the mark out of line 1. A lone CR
/// becomes an LF of the same length, and the mark is dropped, so line and column match the
/// contract and every other offset only moves by the mark.
fn display_text(text: &str) -> (std::borrow::Cow<'_, str>, usize) {
    let (body, shift) = match text.strip_prefix('\u{feff}') {
        Some(rest) => (rest, '\u{feff}'.len_utf8()),
        None => (text, 0),
    };
    let bytes = body.as_bytes();
    let has_lone_cr = bytes
        .iter()
        .enumerate()
        .any(|(i, &b)| b == b'\r' && bytes.get(i + 1) != Some(&b'\n'));
    if !has_lone_cr {
        return (std::borrow::Cow::Borrowed(body), shift);
    }
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' && chars.peek() != Some(&'\n') {
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    (std::borrow::Cow::Owned(out), shift)
}

fn render_one(renderer: &Renderer, source: SourceFile<'_>, d: &Diagnostic) -> String {
    let (text, shift) = display_text(source.text);
    let moved = d.span.start.saturating_sub(shift)..d.span.end.saturating_sub(shift);
    let span = clamp(&moved, &text);
    let mut report = vec![
        Level::ERROR
            .primary_title(d.message.clone())
            .id(d.code.as_str())
            .element(
                Snippet::source(text.as_ref())
                    .path(source.name)
                    .annotation(AnnotationKind::Primary.span(span).label(d.label.clone())),
            ),
    ];
    if let Some(help) = &d.help {
        report.push(Group::with_title(Level::HELP.secondary_title(help.clone())));
    }
    renderer.render(&report)
}

fn clamp(span: &Range<usize>, text: &str) -> Range<usize> {
    let start = span.start.min(text.len());
    let end = span.end.clamp(start, text.len());
    start..end
}

/// The candidate closest to `word`, when one is close enough to be a typo of it.
#[must_use]
pub fn closest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .into_iter()
        .map(|c| (strsim::jaro_winkler(word, c), c))
        .filter(|(score, _)| *score >= 0.8)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_finds_a_typo_and_ignores_unrelated_words() {
        assert_eq!(closest("severty", ["severity", "weight"]), Some("severity"));
        assert_eq!(closest("zzzzzz", ["severity", "weight"]), None);
    }

    #[test]
    fn pointer_escapes_slashes_and_tildes() {
        let d = Diagnostic {
            code: Code::UnknownKey,
            message: String::new(),
            label: String::new(),
            span: 0..0,
            help: None,
            path: vec!["rules".into(), "a/b~c".into()],
            data: None,
        };
        assert_eq!(d.pointer(), "/rules/a~1b~0c");
    }

    #[test]
    fn every_code_keeps_its_published_text_and_is_registered() {
        let published = [
            (Code::Io, "config-io"),
            (Code::TooLarge, "config-too-large"),
            (Code::StyleTooLarge, "style-too-large"),
            (Code::DocumentTooLarge, "document-too-large"),
            (Code::Parse, "config-parse"),
            (Code::UnknownKey, "config-unknown-key"),
            (Code::InvalidType, "config-invalid-type"),
            (Code::InvalidValue, "config-invalid-value"),
            (Code::MissingKey, "config-missing-key"),
            (Code::LegacyVersion, "config-legacy-version"),
            (Code::UnsupportedVersion, "config-unsupported-version"),
            (Code::ExtendsCycle, "config-extends-cycle"),
            (Code::BudgetExceeded, "config-budget-exceeded"),
            (Code::ExtendsMissing, "config-extends-missing"),
            (Code::DuplicateScope, "config-duplicate-scope"),
            (
                Code::FrontmatterFieldDenied,
                "config-frontmatter-field-denied",
            ),
        ];
        let registered = crate::messages::error_entries();
        for (code, text) in published {
            assert_eq!(code.as_str(), text);
            assert!(registered.iter().any(|e| e.code == text), "{text}");
        }
        assert_eq!(registered.len(), published.len());
    }

    #[test]
    fn rendering_tolerates_a_span_past_the_text() {
        let d = Diagnostic {
            code: Code::Parse,
            message: "boom".into(),
            label: "here".into(),
            span: 50..60,
            help: None,
            path: vec![],
            data: None,
        };
        let out = render(
            SourceFile {
                name: "x.toml",
                text: "a = 1\n",
            },
            &[d],
        );
        assert!(out.contains("config-parse"));
    }
}
