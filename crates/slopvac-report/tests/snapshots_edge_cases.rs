//! SARIF, rdjsonl and Code Climate output for the shapes the all-formats snapshot leaves out:
//! an empty report, a document outside the root, a rule without a help
//! URI, a fix with several edits, a multi-line message, and spans over several lines or empty.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::collections::HashMap;

use serde_json::{Value, json};
use slopvac_model::Report;
use slopvac_report::{Format, RenderOptions, render_with};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/v3/conformance");

const OUTSIDE: &str = "../outside/notes é.md";

fn read(name: &str) -> Value {
    let path = format!("{ROOT}/lint-report/valid/{name}.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn finding(rule: &str, category: &str, severity: &str, message: &str, at: [u64; 6]) -> Value {
    let [start, end, line, column, end_line, end_column] = at;
    json!({
        "rule_id": rule,
        "pack": rule.split_once(':').unwrap().0,
        "category": category,
        "severity": severity,
        "message": message,
        "span": {"start": start, "end": end},
        "line": line,
        "column": column,
        "end_line": end_line,
        "end_column": end_column,
        "contexts": ["prose"],
        "ai_signal": "weak",
        "matched_text": "x",
    })
}

/// Two documents. The first sits outside the root and holds, in order: an error of a plugin rule
/// (no help URI) with a three-line message over a span that crosses two lines, a warning whose
/// fix has two edits, and a suggestion over an empty span. The second has no findings.
fn edge_report() -> Report {
    let mut base = read("multi-document");
    let mut outside = base["documents"][0].clone();
    outside["path"] = json!(OUTSIDE);
    let mut warning = finding(
        "core:orwell.not-un",
        "orwell",
        "warning",
        "Say it plainly.",
        [0, 6, 1, 1, 1, 6],
    );
    warning["fix"] = json!({"edits": [
        {"span": {"start": 0, "end": 6}, "text": "hi"},
        {"span": {"start": 12, "end": 18}, "text": "monde"},
    ]});
    outside["findings"] = json!([
        finding(
            "alpha:style.example",
            "alpha:style",
            "error",
            "First line.\r\nSecond line.\n\nThird.",
            [12, 29, 2, 1, 3, 11],
        ),
        warning,
        finding(
            "core:orwell.not-un",
            "orwell",
            "suggestion",
            "Consider it.",
            [19, 19, 3, 1, 3, 1],
        ),
    ]);
    outside["total_findings"] = json!(3);
    outside["unchecked"] = json!([]);
    let mut plain = base["documents"][1].clone();
    plain["path"] = json!("plain.md");
    plain["findings"] = json!([]);
    plain["unchecked"] = json!([{
        "code": "spell-check-skipped",
        "level": "info",
        "message": "No dictionary for this document.",
        "path": "plain.md",
    }]);
    base["documents"] = json!([outside, plain]);
    base["notices"] = json!([]);
    serde_json::from_value(base).unwrap()
}

fn render(format: Format, report: &Report) -> String {
    // "héllo 😀", then "wörld", "third line" and "fourth": the byte offsets above follow from it.
    let sources = HashMap::from([(
        OUTSIDE.to_owned(),
        "héllo 😀\nwörld\nthird line\nfourth".to_owned(),
    )]);
    let options = RenderOptions {
        docs_base: Some("https://docs.example/slopvac/".to_owned()),
    };
    render_with(format, report, &sources, &options)
        .unwrap_or_else(|e| panic!("{}: {e}", format.name()))
}

fn empty_report() -> Report {
    serde_json::from_value(read("empty-run")).unwrap()
}

#[rstest::rstest]
#[case::edge_sarif("edge_sarif", Format::Sarif, edge_report())]
#[case::edge_rdjsonl("edge_rdjsonl", Format::Rdjsonl, edge_report())]
#[case::edge_codeclimate("edge_codeclimate", Format::CodeClimate, edge_report())]
#[case::empty_sarif("empty_sarif", Format::Sarif, empty_report())]
#[case::empty_codeclimate("empty_codeclimate", Format::CodeClimate, empty_report())]
fn pins_the_output(#[case] name: &str, #[case] format: Format, #[case] report: Report) {
    insta::assert_snapshot!(name, render(format, &report));
}

// A snapshot trims surrounding whitespace, so it cannot tell no output from a blank line.
#[test]
fn an_empty_report_renders_no_rdjsonl_bytes() {
    assert_eq!(render(Format::Rdjsonl, &empty_report()), "");
}
