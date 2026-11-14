//! Checkstyle XML: one `<file>` per document, one `<error>` per finding.

use std::fmt::Write as _;

use slopvac_model::{Report, Severity};

use crate::escape::xml;

pub(crate) fn render(report: &Report) -> String {
    let mut out =
        String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<checkstyle version=\"8.0\">\n");
    for doc in &report.documents {
        let _ = writeln!(out, "  <file name=\"{}\">", xml(&doc.path));
        for f in &doc.findings {
            let severity = match f.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Suggestion => "info",
            };
            let _ = writeln!(
                out,
                "    <error line=\"{}\" column=\"{}\" severity=\"{severity}\" message=\"{}\" source=\"{}\"/>",
                f.line,
                f.column,
                xml(&f.message),
                xml(f.rule_id.as_str())
            );
        }
        out.push_str("  </file>\n");
    }
    out.push_str("</checkstyle>\n");
    out
}
