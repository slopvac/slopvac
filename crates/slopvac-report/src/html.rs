//! Standalone accessible HTML report (docs/contract/policies.md, 2.3).
//!
//! No scripts and no remote resources. Every dynamic string goes through `escape::xml`.

use std::fmt::Write as _;

use slopvac_model::{Document, Report};

use crate::escape::xml as e;
use crate::messages::{Label, Line};
use crate::text::{score_text, severity_word, verdict_word};

const STYLE: &str = "\
:root{color-scheme:light dark;--bg:#fff;--fg:#1a1a1a;--panel:#f3f3f3;--muted:#4d4d4d;--pass:#0b5d1e;--fail:#a40000;--focus:#0050c8}\
@media (prefers-color-scheme:dark){:root{--bg:#121212;--fg:#ececec;--panel:#1e1e1e;--muted:#b8b8b8;--pass:#7bd88f;--fail:#ff9d9d;--focus:#8ab4ff}}\
body{background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,sans-serif;margin:0 auto;max-width:60rem;padding:1rem}\
.panel{background:var(--panel);border:1px solid var(--muted);padding:.5rem 1rem;margin:1rem 0}\
.pass{color:var(--pass)}.fail{color:var(--fail)}.muted{color:var(--muted)}\
table{border-collapse:collapse;width:100%}th,td{border:1px solid var(--muted);padding:.25rem .5rem;text-align:left}\
a:focus,summary:focus{outline:3px solid var(--focus);outline-offset:2px}\
.skip{position:absolute;left:-999px}.skip:focus{position:static}\
@media (prefers-reduced-motion:reduce){*{animation:none!important;transition:none!important}}";

pub(crate) fn render(report: &Report) -> String {
    let s = &report.summary;
    let verdict = verdict_word(s.passed);
    let class = if s.passed { "pass" } else { "fail" };
    let title = Label::HtmlTitle.as_static();
    let page_title = Line::HtmlPageTitle {
        verdict: verdict.to_owned(),
    };
    let summary = Line::HtmlSummary {
        documents: s.documents,
        findings: s.findings,
        errors: s.errors,
        warnings: s.warnings,
        suggestions: s.suggestions,
        score: score_text(s.score),
        version: e(report.slopvac_version.as_str()),
    };
    let mut out = String::new();
    // ast-grep-ignore: user-text-literal
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <title>{page_title}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n\
         <a class=\"skip\" href=\"#findings\">{}</a>\n<main>\n\
         <h1>{title}</h1>\n\
         <p class=\"{class}\"><strong>{verdict}</strong> &mdash; {summary}</p>\n",
        Label::HtmlSkip.as_static(),
    );
    if !report.notices.is_empty() {
        // ast-grep-ignore: user-text-literal
        let _ = writeln!(
            out,
            "<section class=\"panel\"><h2>{}</h2>\n<ul>",
            Label::HtmlNoticesHeading.as_static()
        );
        for n in &report.notices {
            let _ = writeln!(
                out,
                "<li><code>{}</code>: {}</li>",
                e(n.code.as_str()),
                e(n.message.as_str())
            );
        }
        out.push_str("</ul></section>\n");
    }
    // ast-grep-ignore: user-text-literal
    let _ = writeln!(
        out,
        "<section class=\"panel\" id=\"findings\"><h2>{}</h2>",
        Label::HtmlDocumentsHeading.as_static()
    );
    let _ = writeln!(
        out,
        "<table><caption>{}</caption>\n<thead><tr>{}</tr></thead>\n<tbody>",
        Label::HtmlDocumentsCaption.as_static(),
        header_cells(&[
            Label::ColumnDocument,
            Label::ColumnVerdict,
            Label::ColumnScore,
            Label::ColumnFindings,
        ])
    );
    for doc in &report.documents {
        // ast-grep-ignore: user-text-literal
        let _ = writeln!(
            out,
            "<tr><th scope=\"row\">{}</th><td>{}</td><td>{}</td><td>{}</td></tr>",
            e(doc.path.as_str()),
            verdict_word(doc.gate.is_passed()),
            score_text(doc.score),
            doc.total_findings
        );
    }
    out.push_str("</tbody></table></section>\n");
    for doc in &report.documents {
        document(&mut out, doc);
    }
    out.push_str("</main>\n</body>\n</html>\n");
    out
}

fn header_cells(labels: &[Label]) -> String {
    let mut out = String::new();
    for l in labels {
        // ast-grep-ignore: user-text-literal
        let _ = write!(out, "<th scope=\"col\">{}</th>", l.as_static());
    }
    out
}

fn document(out: &mut String, doc: &Document) {
    let v = verdict_word(doc.gate.is_passed());
    // ast-grep-ignore: user-text-literal
    let _ = writeln!(
        out,
        "<details class=\"panel\"{}><summary>{} &mdash; {v}</summary>",
        if doc.gate.is_passed() { "" } else { " open" },
        e(doc.path.as_str())
    );
    if !doc.gate.reasons().is_empty() || !doc.unchecked.is_empty() {
        out.push_str("<ul>\n");
        for r in doc.gate.reasons() {
            let _ = writeln!(out, "<li>{}</li>", e(r));
        }
        for n in &doc.unchecked {
            let _ = writeln!(
                out,
                "<li>{}</li>",
                Line::HtmlUnchecked {
                    code: e(n.code.as_str()),
                    message: e(n.message.as_str()),
                }
            );
        }
        out.push_str("</ul>\n");
    }
    if doc.findings.is_empty() {
        let _ = writeln!(
            out,
            "<p class=\"muted\">{}</p>",
            Label::HtmlNoFindings.as_static()
        );
    } else {
        let _ = writeln!(
            out,
            "<table><caption>{}</caption>\n<thead><tr>{}</tr></thead>\n<tbody>",
            Line::HtmlFindingsCaption {
                path: e(doc.path.as_str()),
            },
            header_cells(&[
                Label::ColumnLocation,
                Label::ColumnSeverity,
                Label::ColumnRule,
                Label::ColumnMessage,
            ])
        );
        for f in &doc.findings {
            let _ = writeln!(
                out,
                "<tr><td>{}:{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
                f.start.line(),
                f.start.column(),
                severity_word(f.severity),
                e(f.rule_id.as_str()),
                e(f.message.as_str())
            );
        }
        out.push_str("</tbody></table>\n");
    }
    out.push_str("</details>\n");
}
