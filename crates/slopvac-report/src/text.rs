//! Plain text output: `path:line:column severity rule-id message`.
//!
//! Every dynamic string goes through [`slopvac_sanitize::line`], so a file name
//! or message cannot inject terminal controls or forge output lines.

use std::fmt::Write as _;

use slopvac_model::{Document, Report, Severity};
use slopvac_sanitize::line;

pub(crate) const fn severity_word(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Suggestion => "suggestion",
    }
}

pub(crate) fn render(report: &Report) -> String {
    let mut out = String::new();
    for doc in &report.documents {
        document(&mut out, doc);
    }
    for notice in &report.notices {
        let _ = writeln!(
            out,
            "notice[{}]: {}",
            line(notice.code.as_str()),
            line(notice.message.as_str())
        );
    }
    let s = &report.summary;
    let _ = writeln!(
        out,
        "{} documents, {} findings ({} errors, {} warnings, {} suggestions), score {:.1}: {}",
        s.documents,
        s.findings,
        s.errors,
        s.warnings,
        s.suggestions,
        f64::from(s.score),
        if s.passed { "PASS" } else { "FAIL" }
    );
    out
}

fn document(out: &mut String, doc: &Document) {
    let path = line(doc.path.as_str());
    for f in &doc.findings {
        let _ = writeln!(
            out,
            "{path}:{}:{} {} {} {}",
            f.start.line(),
            f.start.column(),
            severity_word(f.severity),
            line(f.rule_id.as_str()),
            line(f.message.as_str())
        );
    }
    for n in &doc.unchecked {
        let _ = writeln!(
            out,
            "{path}: UNCHECKED [{}] {}",
            line(n.code.as_str()),
            line(n.message.as_str())
        );
    }
    let verdict = if doc.gate.is_passed() { "PASS" } else { "FAIL" };
    let _ = writeln!(out, "{path}: {verdict} score {:.1}", f64::from(doc.score));
    for reason in doc.gate.reasons() {
        let _ = writeln!(out, "{path}: {}", line(reason));
    }
}
