//! What the server asks of the lint engine, and the adapter for `slopvac-engine`.

use std::collections::HashMap;

use frontmatter_spans::detect;
use slopvac_engine::{Engine, EngineError, Input, Kind, Settings};
use slopvac_model::{BomMode, Finding, Notice, RuleId, Span};
use slopvac_parse::{BlockKind, parse_markdown};
use slopvac_rules::RuleSet;

use crate::binary_host::{is_binary_host, unchecked_notice};

/// Why a buffer could not be linted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LintError(pub String);

/// What linting a buffer produced.
#[derive(Debug, Clone, Default)]
pub struct Linted {
    /// Findings, with spans and fixes.
    pub findings: Vec<Finding>,
    /// What did not run, and why (`Document.unchecked`). An editor must not show a buffer
    /// as clean when part of the check was skipped.
    pub unchecked: Vec<Notice>,
    /// Root notices of the run, which leave it complete. The server publishes only
    /// `extract-unsupported` (a diagram type the engine does not read), at the start of the
    /// document because a notice has no position. It drops any other code when it builds
    /// diagnostics, so this field can hold more than the editor shows.
    pub notices: Vec<Notice>,
}

/// Lints the text of one editor buffer.
///
/// `path` is the document path for the report and picks the file kind. `text` is the buffer
/// exactly as the client sent it, so every span is a byte offset into that string, with a
/// leading U+FEFF counted (`docs/contract/positions.md`, LSP section).
pub trait Linter: Send + Sync {
    /// Findings for `text`, with spans and fixes, and the notices about what did not run.
    fn lint(&self, path: &str, text: &str) -> Result<Linted, LintError>;

    /// The exception names `rule` lists, which `reason=` in an allow directive may use, for
    /// a directive inserted on its own line at byte offset `site` of `text`, the start of
    /// the line it would cover. Empty when the engine would not read a directive there:
    /// the document's format has no directive syntax the engine reads (only Markdown does),
    /// or `site` is in front matter or a fenced code block.
    fn allow_reasons(&self, path: &str, text: &str, site: usize, rule: &RuleId) -> Vec<String>;
}

/// A problem found by a [`Check`], shown as a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Byte range in the buffer text.
    pub span: Span,
    /// Severity.
    pub severity: slopvac_model::Severity,
    /// Stable code, shown as the diagnostic code.
    pub code: String,
    /// Message.
    pub message: String,
}

/// An extra check that runs next to the linter on each buffer, such as the terms check or
/// front-matter schema validation.
pub trait Check: Send + Sync {
    /// Problems in `text`. Spans are byte offsets into `text`.
    fn check(&self, path: &str, text: &str) -> Vec<Problem>;
}

/// [`Linter`] over a [`slopvac_engine::Engine`].
pub struct EngineLinter {
    engine: Engine,
    reasons: HashMap<RuleId, Vec<String>>,
}

/// The exception names of each rule in `sets`, for the allow-directive code action.
#[must_use]
pub fn exception_names<'a>(
    sets: impl IntoIterator<Item = &'a RuleSet>,
) -> HashMap<RuleId, Vec<String>> {
    sets.into_iter()
        .flat_map(RuleSet::rules)
        .map(|loaded| {
            let names = loaded
                .rule()
                .exceptions()
                .iter()
                .map(|e| e.name.to_string())
                .collect();
            (loaded.id().clone(), names)
        })
        .collect()
}

impl EngineLinter {
    /// Build the engine from `sets` and `settings`, and record the exception names of each
    /// rule for the allow-directive code action.
    pub fn new(sets: Vec<RuleSet>, settings: Settings) -> Result<Self, EngineError> {
        let reasons = exception_names(&sets);
        Ok(Self {
            engine: Engine::new(sets, settings)?,
            reasons,
        })
    }

    /// Serve an engine that the caller built, with the exception names of its rules (see
    /// [`exception_names`]). The caller sets everything the engine needs, such as terms and
    /// the blocklist.
    #[must_use]
    pub fn from_engine(engine: Engine, reasons: HashMap<RuleId, Vec<String>>) -> Self {
        Self { engine, reasons }
    }
}

/// Whether the directive scanner reads a directive on the line that starts at `site`: not
/// in front matter and not in a code block, where directives are examples.
fn scanner_reads_line(text: &str, site: usize) -> bool {
    let front = detect(text);
    let body_start = front.as_ref().map_or(0, |b| b.body_start);
    if site < body_start {
        return false;
    }
    let Ok(body) = Span::new(body_start, text.len()) else {
        return false;
    };
    // A document that does not parse has no findings to allow.
    let Ok(parsed) = parse_markdown(text, body) else {
        return false;
    };
    !parsed.blocks.iter().any(|block| {
        matches!(block.kind, BlockKind::CodeBlock { .. })
            && block.span.start() <= site
            && site < block.span.end()
    })
}

/// The kind of file at `path`.
fn kind_of(path: &str) -> Kind {
    match path.rsplit('.').next() {
        Some("mdx") => Kind::Mdx,
        Some("rst") => Kind::Rst,
        _ => Kind::Markdown,
    }
}

impl Linter for EngineLinter {
    fn lint(&self, path: &str, text: &str) -> Result<Linted, LintError> {
        if is_binary_host(path) {
            return Ok(Linted {
                findings: Vec::new(),
                unchecked: unchecked_notice().into_iter().collect(),
                notices: Vec::new(),
            });
        }
        let input = Input::new(path, text)
            .with_kind(kind_of(path))
            .with_bom(BomMode::Buffer);
        self.engine
            .lint_with_notices(&input)
            .map(|(document, notices)| Linted {
                findings: document.findings,
                unchecked: document.unchecked,
                notices,
            })
            .map_err(|e| LintError(e.to_string()))
    }

    fn allow_reasons(&self, path: &str, text: &str, site: usize, rule: &RuleId) -> Vec<String> {
        // The engine's directive scanner reads `<!-- slopvac-allow: ... -->` only. MDX
        // rejects that comment and RST has no equivalent, so no action is offered there.
        if kind_of(path) != Kind::Markdown || !scanner_reads_line(text, site) {
            return Vec::new();
        }
        self.reasons.get(rule).cloned().unwrap_or_default()
    }
}
