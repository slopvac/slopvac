//! STUB(slopvac-8l4.12.12): stands in for `slopvac-engine` until it lands.
//!
//! The CLI codes against [`Linter`]. The stub reads each target as bytes,
//! rejects invalid UTF-8 as an incomplete run, and reports no findings.

use std::path::{Path, PathBuf};

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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutcome {
    /// Documents checked.
    pub documents: usize,
    /// Whether at least one gate failed.
    pub gate_failed: bool,
    /// Documents that could not be checked.
    pub incomplete: Vec<CliError>,
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
        if let Some(bytes) = &request.stdin {
            check(redact::STDIN_NAME, bytes, &mut outcome);
        }
        for path in &request.targets {
            let shown = shown_path(path, &request.root);
            match std::fs::read(path) {
                Ok(bytes) => check(&shown, &bytes, &mut outcome),
                Err(err) => outcome.incomplete.push(CliError::new(
                    "input-unreadable",
                    format!("{shown}: {}", err.kind()),
                )),
            }
        }
        outcome
    }
}

fn shown_path(path: &Path, root: &Path) -> String {
    if path.is_absolute() {
        redact::path_for_log(path, root)
    } else {
        path.display().to_string()
    }
}

fn check(name: &str, bytes: &[u8], outcome: &mut RunOutcome) {
    if std::str::from_utf8(bytes).is_ok() {
        outcome.documents += 1;
    } else {
        outcome.incomplete.push(CliError::new(
            "input-not-utf8",
            format!("{name}: not valid UTF-8"),
        ));
    }
}
