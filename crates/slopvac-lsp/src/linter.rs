//! What the server asks of the lint engine, and the adapter for `slopvac-engine`.

use std::collections::HashMap;

use slopvac_engine::{Engine, EngineError, Input, Kind, Settings};
use slopvac_model::{BomMode, Finding, RuleId, Span};
use slopvac_rules::RuleSet;

/// Why a buffer could not be linted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LintError(pub String);

/// Lints the text of one editor buffer.
///
/// `path` is the document path for the report and picks the file kind. `text` is the buffer
/// exactly as the client sent it, so every span is a byte offset into that string, with a
/// leading U+FEFF counted (`docs/contract/positions.md`, LSP section).
pub trait Linter: Send + Sync {
    /// Findings for `text`, with spans and fixes.
    fn lint(&self, path: &str, text: &str) -> Result<Vec<Finding>, LintError>;

    /// The exception names `rule` lists, which `reason=` in an allow directive may use.
    fn allow_reasons(&self, rule: &RuleId) -> Vec<String>;
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

impl EngineLinter {
    /// Build the engine from `sets` and `settings`, and record the exception names of each
    /// rule for the allow-directive code action.
    pub fn new(sets: Vec<RuleSet>, settings: Settings) -> Result<Self, EngineError> {
        let reasons = sets
            .iter()
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
            .collect();
        Ok(Self {
            engine: Engine::new(sets, settings)?,
            reasons,
        })
    }
}

impl Linter for EngineLinter {
    fn lint(&self, path: &str, text: &str) -> Result<Vec<Finding>, LintError> {
        let kind = match path.rsplit('.').next() {
            Some("mdx") => Kind::Mdx,
            Some("rst") => Kind::Rst,
            _ => Kind::Markdown,
        };
        let input = Input {
            path,
            source: text,
            kind,
            bom: BomMode::Buffer,
        };
        self.engine
            .lint(&input)
            .map(|document| document.findings)
            .map_err(|e| LintError(e.to_string()))
    }

    fn allow_reasons(&self, rule: &RuleId) -> Vec<String> {
        self.reasons.get(rule).cloned().unwrap_or_default()
    }
}
