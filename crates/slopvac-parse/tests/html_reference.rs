//! Integration tests for html reference.
//! Parity with the Python HTML projection on the documents in `fixtures/html/reference.json`.
//!
//! The fixture comes from `slopvac.analyze._parse_html`. See `fixtures/html/divergences.md`
//! for what the two read differently.

use serde_json::Value;
use slopvac_model::Span;
use slopvac_parse::parse_html;

/// Words of the projected prose and the source text each one maps to.
#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn words(source: &str) -> Vec<(String, String)> {
    let doc = parse_html(source, Span::new(0, source.len()).unwrap()).unwrap();
    let mut out = Vec::new();
    for projection in doc.blocks.iter().filter_map(|b| b.projection.as_ref()) {
        // Masked code is NUL here and absent in Python, which drops the element.
        let chars: Vec<char> = projection.text().chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_whitespace() || chars[i] == '\0' {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '\0' {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let span = projection.span_for_range(start, i);
            out.push((word, source[span.start()..span.end()].to_owned()));
        }
    }
    out
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn cases() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/html/reference.json")).unwrap()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn pairs(case: &Value) -> Vec<(String, String)> {
    case["words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["text"].as_str().unwrap().to_owned(),
                w["src"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn words_and_source_text_match_python() {
    let cases = cases();
    assert!(cases.len() > 30, "the fixture is not empty");
    for case in cases {
        let source = case["source"].as_str().unwrap();
        assert_eq!(words(source), pairs(&case), "{}", case["id"]);
    }
}
