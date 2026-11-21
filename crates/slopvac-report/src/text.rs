//! Plain text output: `path:line:column severity rule-id message`.

use std::fmt::Write as _;

use slopvac_model::{Document, Notice, Report, Score, Severity};
use slopvac_sanitize::line;

use crate::messages::{Label, Line};

pub(crate) const fn severity_word(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => Label::SeverityError,
        Severity::Warning => Label::SeverityWarning,
        Severity::Suggestion => Label::SeveritySuggestion,
    }
    .as_static()
}

pub(crate) const fn verdict_word(passed: bool) -> &'static str {
    if passed { Label::Pass } else { Label::Fail }.as_static()
}

/// A score with one decimal, the way every format shows it.
pub(crate) fn score_text(score: Score) -> String {
    format!("{:.1}", f64::from(score))
}

pub(crate) fn notice_line(notice: &Notice) -> String {
    Line::TextNotice {
        code: line(notice.code.as_str()),
        message: line(notice.message.as_str()),
    }
    .to_string()
}

pub(crate) fn render(report: &Report) -> String {
    let mut out = String::new();
    for doc in &report.documents {
        document(&mut out, doc);
    }
    for notice in &report.notices {
        let _ = writeln!(out, "{}", notice_line(notice));
    }
    let s = &report.summary;
    let _ = writeln!(
        out,
        "{}",
        Line::TextSummary {
            documents: s.documents,
            findings: s.findings,
            errors: s.errors,
            warnings: s.warnings,
            suggestions: s.suggestions,
            score: score_text(s.score),
            verdict: verdict_word(s.passed).to_owned(),
        }
    );
    out
}

fn document(out: &mut String, doc: &Document) {
    let path = line(doc.path.as_str());
    for f in &doc.findings {
        let _ = writeln!(
            out,
            "{}",
            Line::TextFinding {
                path: path.clone(),
                line: f.start.line(),
                column: f.start.column(),
                severity: severity_word(f.severity).to_owned(),
                rule: line(f.rule_id.as_str()),
                message: line(f.message.as_str()),
            }
        );
    }
    for n in &doc.unchecked {
        let _ = writeln!(
            out,
            "{}",
            Line::TextUnchecked {
                path: path.clone(),
                code: line(n.code.as_str()),
                message: line(n.message.as_str()),
            }
        );
    }
    let _ = writeln!(
        out,
        "{}",
        Line::TextDocument {
            path: path.clone(),
            verdict: verdict_word(doc.gate.is_passed()).to_owned(),
            score: score_text(doc.score),
        }
    );
    for reason in doc.gate.reasons() {
        let _ = writeln!(
            out,
            "{}",
            Line::TextReason {
                path: path.clone(),
                reason: line(reason),
            }
        );
    }
}
