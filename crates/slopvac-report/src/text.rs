//! Plain text output: `path:line:column severity rule-id message`.

use std::fmt::Write as _;

use slopvac_model::{Document, Report, Severity};

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
        writeln!(out, "notice[{}]: {}", notice.code, notice.message).ok();
    }
    let s = &report.summary;
    writeln!(
        out,
        "{} documents, {} findings ({} errors, {} warnings, {} suggestions), score {:.1}: {}",
        s.documents,
        s.findings,
        s.errors,
        s.warnings,
        s.suggestions,
        f64::from(s.score),
        if s.passed { "PASS" } else { "FAIL" }
    )
    .ok();
    out
}

fn document(out: &mut String, doc: &Document) {
    for f in &doc.findings {
        writeln!(
            out,
            "{}:{}:{} {} {} {}",
            doc.path,
            f.start.line(),
            f.start.column(),
            severity_word(f.severity),
            f.rule_id,
            f.message
        )
        .ok();
    }
    for n in &doc.unchecked {
        writeln!(out, "{}: UNCHECKED [{}] {}", doc.path, n.code, n.message).ok();
    }
    let verdict = if doc.gate.is_passed() { "PASS" } else { "FAIL" };
    writeln!(
        out,
        "{}: {verdict} score {:.1}",
        doc.path,
        f64::from(doc.score)
    )
    .ok();
    for reason in doc.gate.reasons() {
        writeln!(out, "{}: {reason}", doc.path).ok();
    }
}
