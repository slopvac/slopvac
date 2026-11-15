//! STUB(slopvac-8l4.12.12): stands in for `slopvac-engine` until it lands.
//!
//! The CLI codes against [`Linter`]. The stub reads each target as bytes,
//! rejects invalid UTF-8 as an incomplete run, and reports no findings. Its
//! [`Report`] lists every readable document with its word count and a clean score.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::json;
use slopvac_model::{KebabName, NonEmptyString, Notice, NoticeLevel, Report};

use crate::errors::CliError;
use crate::redact;
use crate::stdin::StdinPlan;

/// What the CLI asks the engine to lint.
#[derive(Debug, Clone, Default)]
pub struct LintRequest {
    /// Project root. Paths outside it are redacted in messages.
    pub root: PathBuf,
    /// Files to lint. Empty with `stdin` set means standard input.
    pub targets: Vec<PathBuf>,
    /// Raw stdin bytes, when `--stdin` was given.
    pub stdin: Option<Vec<u8>>,
    /// Parser name and mode for the stdin document, set with `stdin`.
    pub stdin_plan: Option<StdinPlan>,
}

/// The result of one run.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    /// Documents checked.
    pub documents: usize,
    /// Whether at least one gate failed.
    pub gate_failed: bool,
    /// Documents that could not be checked.
    pub incomplete: Vec<CliError>,
    /// The v3 report for the documents that were checked.
    pub report: Report,
    /// Text of each checked document by report path, for formats that need columns.
    pub sources: HashMap<String, String>,
}

impl Default for RunOutcome {
    fn default() -> Self {
        Self {
            documents: 0,
            gate_failed: false,
            incomplete: Vec::new(),
            report: clean_report(&[]),
            sources: HashMap::new(),
        }
    }
}

/// A report with no findings for documents given as `(path, words)`.
fn clean_report(docs: &[(String, usize)]) -> Report {
    let register = json!({
        "strong": {"findings": 0, "errors": 0, "warnings": 0, "per_100_words": 0.0},
        "weak": {"findings": 0, "errors": 0, "warnings": 0, "per_100_words": 0.0},
    });
    let prose = json!({"findings": 0, "errors": 0, "warnings": 0, "per_100_words": 0.0});
    let documents: Vec<_> = docs
        .iter()
        .map(|(path, words)| {
            json!({
                "path": path, "profile": "normal",
                "genre": {"id": "reference", "source": "default", "reasons": []},
                "contexts_seen": ["prose"], "config_sources": [{"layer": "core"}],
                "words": words, "sentences": 0, "paragraphs": 0, "findings": [],
                "categories": [], "total_findings": 0, "errors": 0, "warnings": 0,
                "suggestions": 0, "per_100_words": 0.0, "score": 100.0,
                "gate": {"passed": true, "reasons": []}, "unchecked": [],
                "ai_register": register, "prose": prose,
            })
        })
        .collect();
    let words: usize = docs.iter().map(|(_, w)| w).sum();
    serde_json::from_value(json!({
        "schema_version": 3,
        "slopvac_version": env!("CARGO_PKG_VERSION"),
        "summary": {
            "documents": docs.len(), "words": words, "findings": 0, "errors": 0,
            "warnings": 0, "suggestions": 0, "per_100_words": 0.0, "score": 100.0,
            "passed": true, "categories": [], "ai_register": register, "prose": prose,
        },
        "documents": documents,
        "notices": [],
    }))
    .expect("the stub report matches the v3 model")
}

/// The engine contract the CLI depends on.
pub trait Linter {
    /// Lint the request.
    fn lint(&self, request: &LintRequest) -> RunOutcome;
}

/// A linter that finds nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct StubLinter;

impl Linter for StubLinter {
    fn lint(&self, request: &LintRequest) -> RunOutcome {
        let mut outcome = RunOutcome::default();
        let mut docs = Vec::new();
        let mut skipped = Vec::new();
        if let Some(bytes) = &request.stdin {
            check(
                redact::STDIN_NAME,
                bytes,
                &mut outcome,
                &mut docs,
                &mut skipped,
            );
        }
        for path in &request.targets {
            let shown = redact::shown_path(path, &request.root);
            match std::fs::read(path) {
                Ok(bytes) => check(&shown, &bytes, &mut outcome, &mut docs, &mut skipped),
                Err(err) => skip(
                    "input-unreadable",
                    &shown,
                    format!("{shown}: {}", err.kind()),
                    &mut outcome,
                    &mut skipped,
                ),
            }
        }
        outcome.report = clean_report(&docs);
        outcome.report.notices = skipped;
        outcome
    }
}

/// Record a skipped input as an incomplete-run error and a report notice.
fn skip(
    code: &'static str,
    shown: &str,
    message: String,
    outcome: &mut RunOutcome,
    notices: &mut Vec<Notice>,
) {
    let error = CliError::new(code, message);
    notices.push(Notice {
        code: KebabName::new(code).expect("skip codes are kebab-case"),
        level: NoticeLevel::Warning,
        message: NonEmptyString::new(error.message.clone()).expect("messages are not empty"),
        path: Some(shown.to_owned()),
    });
    outcome.incomplete.push(error);
}

fn check(
    name: &str,
    bytes: &[u8],
    outcome: &mut RunOutcome,
    docs: &mut Vec<(String, usize)>,
    notices: &mut Vec<Notice>,
) {
    if let Ok(text) = std::str::from_utf8(bytes) {
        outcome.documents += 1;
        docs.push((name.to_owned(), text.split_whitespace().count()));
        outcome.sources.insert(name.to_owned(), text.to_owned());
    } else {
        skip(
            "input-not-utf8",
            name,
            format!("{name}: not valid UTF-8"),
            outcome,
            notices,
        );
    }
}
