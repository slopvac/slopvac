//! What the server asks of the lint engine, and the adapter for `slopvac-engine`.

use std::collections::HashMap;
use std::path::PathBuf;

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

    /// Read the config again. The server calls it after a config file was saved or changed
    /// on disk, then lints every open buffer again. `open` holds the paths of the buffers that
    /// are open now, so a reload checks the config of those folders and no others. The default
    /// has no config to read.
    fn reload(&self, _open: &[String]) -> Reloaded {
        Reloaded::default()
    }

    /// Absolute paths of the files the active configs were read from, which the server
    /// watches: every config in use, the files they extend and an extended file that does
    /// not exist yet. The default has none. The list can change after a reload or after a
    /// buffer is linted, and the server registers the new list.
    fn watched(&self) -> Vec<String> {
        Vec::new()
    }

    /// Warnings found since the last call, about configs the linter loaded while it linted,
    /// such as the config of a nested folder. The server shows each as a warning message.
    /// The default has none.
    fn take_notices(&self) -> Vec<Notice> {
        Vec::new()
    }

    /// Whether a change to the file at `path`, an absolute path as the client names it,
    /// concerns a config file the linter reads. The default compares with
    /// [`Linter::watched`] by equality. An implementation that knows how paths relate, such
    /// as `a/../b` and `b`, overrides it.
    fn watches(&self, path: &str) -> bool {
        self.watched().iter().any(|w| w == path)
    }
}

/// Why the project config could not be read on a reload.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConfigFailure {
    /// The stable error code, the one `slopvac lint` reports for the same failure.
    pub code: String,
    /// The error text, without the source excerpt.
    pub message: String,
    /// The config file that failed, as an absolute path. It is not always text.
    pub path: PathBuf,
    /// The text of that file and the byte offset in it where the error is, when it has a
    /// position. The server converts the offset to a line and column in the encoding the
    /// client negotiated.
    pub position: Option<(String, usize)>,
}

impl ConfigFailure {
    /// A failure in the file at `path` with no position.
    #[must_use]
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path: path.into(),
            position: None,
        }
    }

    /// The same failure at byte `offset` of `text`, the text of the file.
    #[must_use]
    pub fn at_offset(mut self, text: impl Into<String>, offset: usize) -> Self {
        self.position = Some((text.into(), offset));
        self
    }
}

/// What a [`Linter::reload`] found.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Reloaded {
    /// Warnings about the config that was read, such as an unknown rule id.
    pub notices: Vec<Notice>,
    /// Set when the config could not be read. Until a later reload succeeds, the linter
    /// reports every buffer as not linted.
    pub failure: Option<ConfigFailure>,
}

impl Reloaded {
    /// What a reload found: the config `notices`, and the `failure` when the config could
    /// not be read.
    #[must_use]
    pub fn new(notices: Vec<Notice>, failure: Option<ConfigFailure>) -> Self {
        Self { notices, failure }
    }
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

    /// The engine behind the linter.
    #[must_use]
    pub const fn engine(&self) -> &Engine {
        &self.engine
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

impl EngineLinter {
    /// Lint `text` as `Linter::lint` does, for a document whose path relative to its config
    /// root is `relative` and whose front matter declares `genre`. The relative path is what
    /// the `files` globs of `[[genres]]` blocks match, and the declared genre outranks them.
    pub fn lint_resolved(
        &self,
        path: &str,
        text: &str,
        relative: Option<&str>,
        genre: Option<&str>,
    ) -> Result<Linted, LintError> {
        if is_binary_host(path) {
            return Ok(Linted {
                findings: Vec::new(),
                unchecked: unchecked_notice().into_iter().collect(),
                notices: Vec::new(),
            });
        }
        let mut input = Input::new(path, text)
            .with_kind(kind_of(path))
            .with_bom(BomMode::Buffer);
        if let Some(relative) = relative {
            input = input.with_relative_path(relative);
        }
        if let Some(genre) = genre {
            input = input.with_declared_genre(genre);
        }
        self.engine
            .lint_with_notices(&input)
            .map(|(document, notices)| Linted {
                findings: document.findings,
                unchecked: document.unchecked,
                notices,
            })
            .map_err(|e| LintError(e.to_string()))
    }
}

impl Linter for EngineLinter {
    fn lint(&self, path: &str, text: &str) -> Result<Linted, LintError> {
        self.lint_resolved(path, text, None, None)
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
