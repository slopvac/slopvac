//! `JUnit` XML. The schema has no line or column attribute, so the position goes in the message.
//!
//! Failures follow the document gate. Each document has one `gate` test case that fails when
//! the gate failed, with the reasons as its message. Findings are separate test cases that
//! pass and carry the diagnostic as `system-out`, so a warning under the threshold never
//! fails a build.

use std::fmt::Write as _;

use slopvac_model::{Document, Report};

use crate::escape::{xml, xml_attr};
use crate::text::severity_word;

pub(crate) fn render(report: &Report) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let tests: usize = report.documents.iter().map(|d| d.findings.len() + 1).sum();
    let failures = report
        .documents
        .iter()
        .filter(|d| !d.gate.is_passed())
        .count();
    // ast-grep-ignore: user-text-literal
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

fn suite(out: &mut String, doc: &Document) {
    let path = xml_attr(doc.path.as_str());
    // ast-grep-ignore: user-text-literal
    let _ = writeln!(
        out,
        "  <testsuite name=\"{path}\" tests=\"{}\" failures=\"{}\">",
        doc.findings.len() + 1,
        usize::from(!doc.gate.is_passed())
    );
    for f in &doc.findings {
        // ast-grep-ignore: user-text-literal
        let _ = write!(
            out,
            "    <testcase classname=\"{path}\" name=\"{} {}:{}\">",
            xml_attr(f.rule_id.as_str()),
            f.start.line(),
            f.start.column()
        );
        let _ = write!(
            out,
            "<system-out>{}:{} {} {}: {}</system-out>",
            f.start.line(),
            f.start.column(),
            severity_word(f.severity),
            xml(f.rule_id.as_str()),
            xml(f.message.as_str())
        );
        out.push_str("</testcase>\n");
    }
    // ast-grep-ignore: user-text-literal
    let _ = write!(out, "    <testcase classname=\"{path}\" name=\"gate\">");
    if !doc.gate.is_passed() {
        let reasons = doc.gate.reasons().join("; ");
        // ast-grep-ignore: user-text-literal
        let _ = write!(
            out,
            "<failure type=\"gate\" message=\"{}\">{}</failure>",
            xml_attr(&reasons),
            xml(&reasons)
        );
    }
    out.push_str("</testcase>\n  </testsuite>\n");
}
