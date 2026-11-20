//! The result of linting one buffer, in the shape the LSP handlers use.

use std::fmt::Write as _;
use std::sync::Arc;

use slopvac_messages::{Catalog, render};
use slopvac_model::{Edit, Finding, Notice, NoticeLevel, RuleId, Severity, Span};

use crate::binary_host::is_binary_host;
use crate::linter::{Check, Linter, Problem};
use crate::messages::LspMessage;

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
        let mut hover = render(&LspMessage::HoverFinding {
            rule: f.rule_id.to_string(),
            severity: format!("{:?}", f.severity),
            category: f.category.to_string(),
            message: f.message.to_string(),
        });
        if !f.matched_text.is_empty() {
            let matched = render(&LspMessage::HoverMatched {
                text: f.matched_text.replace('`', "'"),
            });
            let _ = write!(hover, "\n\n{matched}");
        }
        if let Some(fix) = &f.fix {
            let texts: Vec<String> = fix
                .edits()
                .iter()
                .map(|e| format!("`{}`", e.text.replace('`', "'")))
                .collect();
            let fix = render(&LspMessage::HoverFix {
                replacements: texts.join(", "),
            });
            let _ = write!(hover, "\n\n{fix}");
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
        let hover = render(&LspMessage::HoverProblem {
            code: p.code.clone(),
            message: p.message.clone(),
        });
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
        let message = render(&LspMessage::Unchecked {
            message: n.message.to_string(),
        });
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
        let notice = LspMessage::LintIncomplete {
            reason: reason.to_owned(),
        };
        let message = render(&notice);
        Self {
            span: None,
            line: 0,
            severity: Severity::Warning,
            code: notice.code().to_owned(),
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
    // A binary container is not text: its checks would read bytes as prose.
    for check in checks.iter().filter(|_| !is_binary_host(path)) {
        items.extend(check.check(path, text).into_iter().map(Item::from_problem));
    }
    Analysis {
        generation,
        version,
        items,
    }
}
