//! The result of linting one buffer, in the shape the LSP handlers use.
#![expect(
    clippy::let_underscore_must_use,
    reason = "fmt::Write for String never returns an error"
)]

use std::fmt::Write as _;
use std::sync::Arc;

use slopvac_model::{Edit, Finding, Notice, NoticeLevel, RuleId, Severity, Span};

use crate::linter::{Check, Linter, Problem};

/// One diagnostic-worthy item: a finding, a check problem or a failure notice.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    /// Byte range; `None` for a whole-document item, which sits at `line`.
    pub(crate) span: Option<Span>,
    /// Zero-based line of a whole-document item.
    pub(crate) line: usize,
    pub(crate) severity: Severity,
    pub(crate) code: String,
    pub(crate) message: String,
    /// Edits of the finding's fix, in buffer offsets.
    pub(crate) fix: Option<Vec<Edit>>,
    /// The rule an allow directive can name.
    pub(crate) rule: Option<RuleId>,
    /// Markdown for hover.
    pub(crate) hover: String,
}

/// Items for one version of one buffer.
#[derive(Debug)]
pub(crate) struct Analysis {
    /// The generation of the document text that was linted.
    pub(crate) generation: u64,
    /// The client's version of that text.
    pub(crate) version: i32,
    pub(crate) items: Vec<Item>,
}

impl Item {
    fn from_finding(f: Finding) -> Self {
        let mut hover = format!(
            "**{}** ({:?}, category `{}`)\n\n{}",
            f.rule_id, f.severity, f.category, f.message
        );
        if !f.matched_text.is_empty() {
            let _ = write!(hover, "\n\nMatched: `{}`", f.matched_text.replace('`', "'"));
        }
        if let Some(fix) = &f.fix {
            let texts: Vec<String> = fix
                .edits()
                .iter()
                .map(|e| format!("`{}`", e.text.replace('`', "'")))
                .collect();
            let _ = write!(hover, "\n\nFix: replace with {}", texts.join(", "));
        }
        Self {
            span: f.extent.map(|e| e.span),
            line: f.start.line() - 1,
            severity: f.severity,
            code: f.rule_id.to_string(),
            message: f.message.to_string(),
            fix: f.fix.map(|fix| fix.edits().to_vec()),
            rule: Some(f.rule_id),
            hover,
        }
    }

    fn from_problem(p: Problem) -> Self {
        let hover = format!("**{}**\n\n{}", p.code, p.message);
        Self {
            span: Some(p.span),
            line: 0,
            severity: p.severity,
            code: p.code,
            message: p.message,
            fix: None,
            rule: None,
            hover,
        }
    }

    /// A part of the check did not run, such as a skipped rule kind or a missing RST
    /// converter. The buffer must not look fully checked.
    fn from_notice(n: &Notice) -> Self {
        let message = format!("slopvac did not fully check this document: {}", n.message);
        Self {
            span: None,
            line: 0,
            severity: match n.level {
                NoticeLevel::Info => Severity::Suggestion,
                NoticeLevel::Warning => Severity::Warning,
            },
            code: n.code.to_string(),
            hover: message.clone(),
            message,
            fix: None,
            rule: None,
        }
    }

    /// The check could not finish. An unlinted buffer must not look clean (exit 2 semantics).
    fn incomplete(reason: &str) -> Self {
        let message = format!("slopvac could not lint this document: {reason}");
        Self {
            span: None,
            line: 0,
            severity: Severity::Warning,
            code: "lint-incomplete".to_owned(),
            hover: message.clone(),
            message,
            fix: None,
            rule: None,
        }
    }
}

/// Lint `text` and run the extra checks.
pub(crate) fn analyze(
    linter: &dyn Linter,
    checks: &[Arc<dyn Check>],
    path: &str,
    generation: u64,
    version: i32,
    text: &str,
) -> Analysis {
    let mut items: Vec<Item> = match linter.lint(path, text) {
        Ok(linted) => linted
            .findings
            .into_iter()
            .map(Item::from_finding)
            .chain(linted.unchecked.into_iter().map(|n| Item::from_notice(&n)))
            .collect(),
        Err(e) => vec![Item::incomplete(&e.0)],
    };
    for check in checks {
        items.extend(check.check(path, text).into_iter().map(Item::from_problem));
    }
    Analysis {
        generation,
        version,
        items,
    }
}
