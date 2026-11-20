//! Markdown summary for pull request comments and job summaries.

use std::fmt::Write as _;

use slopvac_model::{Document, Report};

use crate::messages::{Label, Line};
use crate::text::{score_text, severity_word, verdict_word};

fn cell(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '|' | '\\' | '`' | '<' | '>' | '&' | '*' | '_' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn render(report: &Report) -> String {
    let s = &report.summary;
    let mut out = format!("{}\n\n", Label::MarkdownTitle.as_static());
    let _ = writeln!(
        out,
        "{}\n",
        Line::MarkdownSummary {
            verdict: verdict_word(s.passed).to_owned(),
            documents: s.documents,
            findings: s.findings,
            errors: s.errors,
            warnings: s.warnings,
            suggestions: s.suggestions,
            score: score_text(s.score),
        }
    );
    if !report.notices.is_empty() {
        let _ = write!(out, "{}\n\n", Label::MarkdownNoticesHeading.as_static());
        for n in &report.notices {
            let _ = writeln!(
                out,
                "{}",
                Line::MarkdownNotice {
                    code: n.code.to_string(),
                    message: cell(n.message.as_str()),
                }
            );
        }
        out.push('\n');
    }
    for doc in &report.documents {
        document(&mut out, doc);
    }
    out
}

fn document(out: &mut String, doc: &Document) {
    let _ = writeln!(
        out,
        "{}\n",
        Line::MarkdownDocumentHeading {
            path: cell(doc.path.as_str()),
            verdict: verdict_word(doc.gate.is_passed()).to_owned(),
        }
    );
    let _ = writeln!(
        out,
        "{}\n",
        Line::MarkdownDocumentStats {
            score: score_text(doc.score),
            words: doc.words,
            findings: doc.total_findings,
        }
    );
    for reason in doc.gate.reasons() {
        let _ = writeln!(
            out,
            "{}",
            Line::MarkdownReason {
                reason: cell(reason)
            }
        );
    }
    for n in &doc.unchecked {
        let _ = writeln!(
            out,
            "{}",
            Line::MarkdownUnchecked {
                code: n.code.to_string(),
                message: cell(n.message.as_str()),
            }
        );
    }
    if !doc.gate.reasons().is_empty() || !doc.unchecked.is_empty() {
        out.push('\n');
    }
    if doc.findings.is_empty() {
        return;
    }
    let _ = writeln!(out, "{}", Label::MarkdownTableHeader.as_static());
    for f in &doc.findings {
        let _ = writeln!(
            out,
            "{}",
            Line::MarkdownRow {
                line: f.start.line(),
                column: f.start.column(),
                severity: severity_word(f.severity).to_owned(),
                rule: cell(f.rule_id.as_str()),
                message: cell(f.message.as_str()),
            }
        );
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::cell;

    #[test]
    fn cells_cannot_break_the_table_or_inject_markup() {
        assert_eq!(
            cell("a|b\n`c` <x> [l](u)"),
            "a\\|b \\`c\\` \\<x\\> \\[l\\](u)"
        );
    }
}
