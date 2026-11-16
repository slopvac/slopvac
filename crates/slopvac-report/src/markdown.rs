//! Markdown summary for pull request comments and job summaries.

use std::fmt::Write as _;

use slopvac_model::{Document, Report};

use crate::text::severity_word;

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
    let mut out = String::from("# slopvac report\n\n");
    let _ = writeln!(
        out,
        "**{}**: {} documents, {} findings ({} errors, {} warnings, {} suggestions), score {:.1}.\n",
        if s.passed { "PASS" } else { "FAIL" },
        s.documents,
        s.findings,
        s.errors,
        s.warnings,
        s.suggestions,
        f64::from(s.score)
    );
    if !report.notices.is_empty() {
        out.push_str("## Notices\n\n");
        for n in &report.notices {
            let _ = writeln!(out, "- `{}`: {}", n.code, cell(n.message.as_str()));
        }
        out.push('\n');
    }
    for doc in &report.documents {
        document(&mut out, doc);
    }
    out
}

fn document(out: &mut String, doc: &Document) {
    let verdict = if doc.gate.is_passed() { "PASS" } else { "FAIL" };
    let _ = writeln!(out, "## {} {verdict}\n", cell(doc.path.as_str()));
    let _ = writeln!(
        out,
        "Score {:.1}, {} words, {} findings.\n",
        f64::from(doc.score),
        doc.words,
        doc.total_findings
    );
    for reason in doc.gate.reasons() {
        let _ = writeln!(out, "- {}", cell(reason));
    }
    for n in &doc.unchecked {
        let _ = writeln!(
            out,
            "- UNCHECKED `{}`: {}",
            n.code,
            cell(n.message.as_str())
        );
    }
    if !doc.gate.reasons().is_empty() || !doc.unchecked.is_empty() {
        out.push('\n');
    }
    if doc.findings.is_empty() {
        return;
    }
    out.push_str("| Location | Severity | Rule | Message |\n| --- | --- | --- | --- |\n");
    for f in &doc.findings {
        let _ = writeln!(
            out,
            "| {}:{} | {} | {} | {} |",
            f.start.line(),
            f.start.column(),
            severity_word(f.severity),
            cell(f.rule_id.as_str()),
            cell(f.message.as_str())
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
