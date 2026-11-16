//! Standalone accessible HTML report (docs/contract/policies.md, 2.3).
//!
//! No scripts and no remote resources. Every dynamic string goes through `escape::xml`.

use std::fmt::Write as _;

use slopvac_model::{Document, Report};

use crate::escape::xml as e;
use crate::text::severity_word;

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
    let verdict = if s.passed { "PASS" } else { "FAIL" };
    let class = if s.passed { "pass" } else { "fail" };
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <title>slopvac report: {verdict}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n\
         <a class=\"skip\" href=\"#findings\">Skip to findings</a>\n<main>\n\
         <h1>slopvac report</h1>\n\
         <p class=\"{class}\"><strong>{verdict}</strong> &mdash; {} documents, {} findings \
         ({} errors, {} warnings, {} suggestions), score {:.1}. slopvac {}.</p>\n",
        s.documents,
        s.findings,
        s.errors,
        s.warnings,
        s.suggestions,
        f64::from(s.score),
        e(report.slopvac_version.as_str())
    );
    if !report.notices.is_empty() {
        out.push_str("<section class=\"panel\"><h2>Notices</h2>\n<ul>\n");
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
    out.push_str("<section class=\"panel\" id=\"findings\"><h2>Documents</h2>\n");
    out.push_str("<table><caption>Result for each document</caption>\n<thead><tr><th scope=\"col\">Document</th><th scope=\"col\">Verdict</th><th scope=\"col\">Score</th><th scope=\"col\">Findings</th></tr></thead>\n<tbody>\n");
    for doc in &report.documents {
        let v = if doc.gate.is_passed() { "PASS" } else { "FAIL" };
        let _ = writeln!(
            out,
            "<tr><th scope=\"row\">{}</th><td>{v}</td><td>{:.1}</td><td>{}</td></tr>",
            e(doc.path.as_str()),
            f64::from(doc.score),
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

fn document(out: &mut String, doc: &Document) {
    let v = if doc.gate.is_passed() { "PASS" } else { "FAIL" };
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
                "<li>UNCHECKED <code>{}</code>: {}</li>",
                e(n.code.as_str()),
                e(n.message.as_str())
            );
        }
        out.push_str("</ul>\n");
    }
    if doc.findings.is_empty() {
        out.push_str("<p class=\"muted\">No findings.</p>\n");
    } else {
        let _ = write!(
            out,
            "<table><caption>Findings in {}</caption>\n<thead><tr><th scope=\"col\">Location</th><th scope=\"col\">Severity</th><th scope=\"col\">Rule</th><th scope=\"col\">Message</th></tr></thead>\n<tbody>\n",
            e(doc.path.as_str())
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
