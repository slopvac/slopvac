//! GitHub Actions workflow-command annotations.

use std::fmt::Write as _;

use slopvac_model::{Report, Severity};

use crate::escape::{github_data, github_property};

pub(crate) fn render(report: &Report) -> String {
    let mut out = String::new();
    for doc in &report.documents {
        for f in &doc.findings {
            let level = match f.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Suggestion => "notice",
            };
            let _ = writeln!(
                out,
                "::{level} file={},line={},col={},title={}::{}",
                github_property(&doc.path),
                f.line,
                f.column,
                github_property(f.rule_id.as_str()),
                github_data(&f.message)
            );
        }
    }
    out
}
