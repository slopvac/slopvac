//! The vocabulary rules against the results that Vale gave 2.x.
//!
//! The golden suite records no vocabulary findings: its reference runs with an empty config,
//! so 2.x sets no word blocklist and runs no vocabulary rule. This oracle stays the only
//! check of the sweep. `fixtures/vocabulary-parity.json` keeps what the
//! 2.x sweep reported with the shipped example blocklist (`fixtures/generate_vocabulary.py`
//! runs Vale 3.27 or later, the oracle). Each document lists the findings of the vocabulary
//! rule only; the rest of the report is the golden's concern. The test compares the native
//! sweep on the Rust tagger with the recorded results: rule, start, end column, severity and
//! matched text.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(
    clippy::allow_attributes_without_reason,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]
#![allow(missing_docs, clippy::assert_is_empty)]

mod common;

use std::path::Path;

use common::{legacy_engine, read_json, root};
use serde_json::Value;
use slopvac_engine::Input;
use slopvac_terms::Blocklist;

const OWNER: &str = "core:ste-words.word-outside-controlled-vocabulary";

type Row = (String, usize, usize, usize, String, String);

fn recorded(finding: &Value) -> Row {
    let number = |k: &str| usize::try_from(finding[k].as_u64().unwrap()).unwrap();
    (
        finding["rule"].as_str().unwrap().to_owned(),
        number("line"),
        number("column"),
        number("end_column"),
        finding["severity"].as_str().unwrap().to_owned(),
        finding["matched"].as_str().unwrap().to_owned(),
    )
}

#[test]
fn the_native_sweep_matches_the_recorded_vale_results() {
    let fixture = read_json(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vocabulary-parity.json"),
    );
    let blocklist =
        Blocklist::load(&root().join("packages/slopvac-lint/examples/blocklist.toml")).unwrap();
    let engine = legacy_engine("normal").with_blocklist(&blocklist).unwrap();
    let mut problems = Vec::new();
    let mut compared = 0;
    for entry in fixture["documents"].as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let text = entry["text"].as_str().unwrap();
        let doc = engine.lint(&Input::markdown(name, text)).unwrap();
        let mut got: Vec<Row> = doc
            .findings
            .iter()
            .filter(|f| f.rule_id.as_str() == OWNER)
            .map(|f| {
                // Vale ends a match that spans lines at the end of its first line.
                let end = f.extent.map_or(f.start, |e| e.end);
                let end_column = if end.line() == f.start.line() {
                    end.column()
                } else {
                    text.lines()
                        .nth(f.start.line() - 1)
                        .map_or(0, |l| l.chars().count())
                        + 1
                };
                (
                    f.rule_id.to_string(),
                    f.start.line(),
                    f.start.column(),
                    end_column,
                    format!("{:?}", f.severity).to_lowercase(),
                    f.matched_text.clone(),
                )
            })
            .collect();
        let mut want: Vec<Row> = entry["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(recorded)
            .collect();
        got.sort();
        want.sort();
        compared += want.len();
        if got != want {
            problems.push(format!("{name}: want {want:?}, got {got:?}"));
        }
    }
    assert!(compared >= 10, "only {compared} recorded findings");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
