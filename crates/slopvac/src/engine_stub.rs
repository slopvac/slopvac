//! STUB(slopvac-8l4.12.12): stands in for `slopvac-engine` until it lands.
//!
//! The CLI codes against [`Linter`]. The stub reads each target as bytes,
//! rejects invalid UTF-8 as an incomplete run, and reports no findings.

use std::path::PathBuf;

/// What the CLI asks the engine to lint.
#[derive(Debug, Clone, Default)]
pub struct LintRequest {
    /// Files to lint. Empty with `stdin` set means standard input.
    pub targets: Vec<PathBuf>,
    /// Raw stdin bytes, when `--stdin` was given.
    pub stdin: Option<Vec<u8>>,
}

/// The result of one run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutcome {
    /// Documents checked.
    pub documents: usize,
    /// Whether at least one gate failed.
    pub gate_failed: bool,
    /// Messages for documents that could not be checked.
    pub incomplete: Vec<String>,
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
            check(crate::redact::STDIN_NAME, bytes, &mut outcome);
        }
        for path in &request.targets {
            match std::fs::read(path) {
                Ok(bytes) => check(&path.display().to_string(), &bytes, &mut outcome),
                Err(err) => outcome
                    .incomplete
                    .push(format!("{}: {}", path.display(), err.kind())),
            }
        }
        outcome
    }
}

fn check(name: &str, bytes: &[u8], outcome: &mut RunOutcome) {
    if std::str::from_utf8(bytes).is_ok() {
        outcome.documents += 1;
    } else {
        outcome.incomplete.push(format!("{name}: not valid UTF-8"));
    }
}
