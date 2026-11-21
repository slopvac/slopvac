//! Every format's full output for one rich report, pinned byte for byte.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::collections::HashMap;

use serde_json::{Value, json};
use slopvac_model::Report;
use slopvac_report::{Format, RenderOptions, render_with};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schemas/v3/conformance/lint-report/valid/multi-document.json"
);

/// Three documents from the conformance fixture (all severities, a fix, an unchecked check,
/// a gate failure, a zero-finding file, a run notice) plus a fourth with non-ASCII text, hostile
/// characters in the message, a fix, and a span that ends at the end of the file.
fn report() -> Report {
    let mut base: Value = serde_json::from_str(&std::fs::read_to_string(FIXTURE).unwrap()).unwrap();
    let mut extra = base["documents"][0].clone();
    extra["path"] = json!("dossiers/ça & <é>.md");
    extra["findings"] = json!([{
        "rule_id": "core:prose-inflation.slop-lexicon",
        "pack": "core",
        "category": "prose-inflation",
        "severity": "warning",
        "message": "Replace \"wörld\" <b>& `x` | y\u{1}",
        "span": {"start": 12, "end": 18},
        "line": 2,
        "column": 7,
        "end_line": 2,
        "end_column": 12,
        "contexts": ["prose"],
        "ai_signal": "weak",
        "matched_text": "wörld",
        "fix": {"edits": [{"span": {"start": 12, "end": 18}, "text": "monde"}]}
    }]);
    extra["total_findings"] = json!(1);
    extra["unchecked"] = json!([]);
    extra["gate"] = json!({"passed": true, "reasons": []});
    base["documents"].as_array_mut().unwrap().push(extra);
    serde_json::from_value(base).unwrap()
}

fn sources() -> HashMap<String, String> {
    let guide = format!("{}\n{}", "a".repeat(300), "b".repeat(300));
    HashMap::from([
        ("docs/guide.md".to_owned(), guide),
        ("README.md".to_owned(), "# Readme\n".to_owned()),
        ("src/app.py".to_owned(), "# comment\n".to_owned()),
        // 12 bytes before "wörld" (6 bytes) which ends the file.
        (
            "dossiers/ça & <é>.md".to_owned(),
            "héllo 😀\nwörld".to_owned(),
        ),
    ])
}

fn rendered(format: Format) -> String {
    let options = RenderOptions {
        docs_base: Some("https://docs.example/slopvac/".to_owned()),
    };
    render_with(format, &report(), &sources(), &options)
        .unwrap_or_else(|e| panic!("{}: {e}", format.name()))
}

macro_rules! snapshot {
    ($name:ident, $format:expr) => {
        #[test]
        fn $name() {
            insta::assert_snapshot!(rendered($format));
        }
    };
}

snapshot!(text, Format::Text);
snapshot!(json, Format::Json);
snapshot!(github, Format::Github);
snapshot!(sarif, Format::Sarif);
snapshot!(rdjsonl, Format::Rdjsonl);
snapshot!(codeclimate, Format::CodeClimate);
snapshot!(junit, Format::Junit);
snapshot!(checkstyle, Format::Checkstyle);
snapshot!(markdown, Format::Markdown);
snapshot!(html, Format::Html);
