//! Diagnostics: coded errors that carry a span, rendered with `annotate-snippets`.

use std::fmt;
use std::ops::Range;

use annotate_snippets::{AnnotationKind, Group, Level, Renderer, Snippet};

use crate::tree::LineIndex;

/// Stable identifier of a diagnostic, printed in brackets and matched by tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
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
    /// An `extends` entry that names no readable config.
    ExtendsMissing,
    /// Two `[[overrides]]` or `[[genres]]` blocks with the same `files` set.
    DuplicateScope,
}

impl Code {
    /// The identifier text, such as `config-unknown-key`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Parse => "config-parse",
            Code::UnknownKey => "config-unknown-key",
            Code::InvalidType => "config-invalid-type",
            Code::InvalidValue => "config-invalid-value",
            Code::MissingKey => "config-missing-key",
            Code::LegacyVersion => "config-legacy-version",
            Code::UnsupportedVersion => "config-unsupported-version",
            Code::ExtendsCycle => "config-extends-cycle",
            Code::ExtendsMissing => "config-extends-missing",
            Code::DuplicateScope => "config-duplicate-scope",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
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

    /// One-based line and column of the span start in `text`.
    #[must_use]
    pub fn location(&self, text: &str) -> (usize, usize) {
        LineIndex::new(text).line_col(self.span.start)
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
#[must_use]
pub fn render(source: SourceFile<'_>, diagnostics: &[Diagnostic]) -> String {
    let renderer = Renderer::plain();
    diagnostics
        .iter()
        .map(|d| render_one(&renderer, source, d))
        .collect::<Vec<_>>()
        .join("\n")
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
        };
        assert_eq!(d.pointer(), "/rules/a~1b~0c");
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
