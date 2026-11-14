//! `JUnit` XML. The schema has no line or column attribute, so the position goes in the message.

use std::fmt::Write as _;

use slopvac_model::{Document, Report, Severity};

use crate::escape::xml;

pub(crate) fn render(report: &Report) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let tests: usize = report.documents.iter().map(|d| d.findings.len() + 1).sum();
    let failures: usize = report.documents.iter().map(failures).sum();
    let _ = writeln!(
        out,
        "<testsuites name=\"slopvac\" tests=\"{tests}\" failures=\"{failures}\">"
    );
    for doc in &report.documents {
        suite(&mut out, doc);
    }
    out.push_str("</testsuites>\n");
    out
}

fn is_failure(severity: Severity) -> bool {
    severity != Severity::Suggestion
}

fn failures(doc: &Document) -> usize {
    doc.findings
        .iter()
        .filter(|f| is_failure(f.severity))
        .count()
        + usize::from(!doc.gate.passed)
}

fn suite(out: &mut String, doc: &Document) {
    let path = xml(&doc.path);
    let _ = writeln!(
        out,
        "  <testsuite name=\"{path}\" tests=\"{}\" failures=\"{}\">",
        doc.findings.len() + 1,
        failures(doc)
    );
    for f in &doc.findings {
        let _ = write!(
            out,
            "    <testcase classname=\"{path}\" name=\"{} {}:{}\">",
            xml(f.rule_id.as_str()),
            f.line,
            f.column
        );
        let message = xml(&format!("{}:{} {}", f.line, f.column, f.message));
        if is_failure(f.severity) {
            let kind = if f.severity == Severity::Error {
                "error"
            } else {
                "warning"
            };
            let _ = write!(
                out,
                "<failure type=\"{kind}\" message=\"{message}\">{path}:{}:{} {}</failure>",
                f.line,
                f.column,
                xml(&f.message)
            );
        } else {
            let _ = write!(out, "<system-out>{message}</system-out>");
        }
        out.push_str("</testcase>\n");
    }
    let _ = write!(out, "    <testcase classname=\"{path}\" name=\"gate\">");
    if !doc.gate.passed {
        let reasons = xml(&doc.gate.reasons.join("; "));
        let _ = write!(
            out,
            "<failure type=\"gate\" message=\"{reasons}\">{reasons}</failure>"
        );
    }
    out.push_str("</testcase>\n  </testsuite>\n");
}
