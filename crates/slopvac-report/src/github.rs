//! GitHub Actions workflow-command annotations.

use std::fmt::Write as _;

use slopvac_model::{Report, Severity};

use slopvac_sanitize::{workflow_data, workflow_property};

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
                workflow_property(doc.path.as_str()),
                f.start.line(),
                f.start.column(),
                workflow_property(f.rule_id.as_str()),
                workflow_data(f.message.as_str())
            );
        }
    }
    out
}
