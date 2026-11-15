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
        let _ = writeln!(out, "notice[{}]: {}", notice.code, notice.message);
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
    for f in &doc.findings {
        let _ = writeln!(
            out,
            "{}:{}:{} {} {} {}",
            doc.path,
            f.start.line(),
            f.start.column(),
            severity_word(f.severity),
            f.rule_id,
            f.message
        );
    }
    for n in &doc.unchecked {
        let _ = writeln!(out, "{}: UNCHECKED [{}] {}", doc.path, n.code, n.message);
    }
    let verdict = if doc.gate.is_passed() { "PASS" } else { "FAIL" };
    let _ = writeln!(
        out,
        "{}: {verdict} score {:.1}",
        doc.path,
        f64::from(doc.score)
    );
    for reason in doc.gate.reasons() {
        let _ = writeln!(out, "{}: {reason}", doc.path);
    }
}
