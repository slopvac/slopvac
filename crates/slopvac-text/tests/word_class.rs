//! The word-class rewrite agrees with Python's `regex` module on generated patterns.
//!
//! `fixtures/word_class_differential.json` holds seeded patterns, Unicode edge-case texts and
//! the spans Python finds (`fixtures/generate_word_class.py`). Python is the oracle; no
//! expected value comes from this crate.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(
    clippy::allow_attributes_without_reason,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]
#![allow(missing_docs, clippy::cast_possible_truncation)]

use serde_json::Value;
use slopvac_text::Pattern;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/word_class_differential.json")).unwrap()
}

/// Code point offsets to byte offsets.
fn bytes(text: &str, at: u64) -> usize {
    text.char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .nth(at as usize)
        .unwrap()
}

#[test]
fn generated_patterns_match_python_on_unicode_edge_texts() {
    let doc = fixture();
    let texts: Vec<&str> = doc["texts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    let patterns = doc["patterns"].as_array().unwrap();
    assert!(patterns.len() >= 500, "{} patterns", patterns.len());
    let mut hits = 0;
    let mut failures = Vec::new();
    for entry in patterns {
        let source = entry["p"].as_str().unwrap();
        let pattern = Pattern::new(source, entry["i"].as_bool().unwrap())
            .unwrap_or_else(|e| panic!("{source:?} must compile: {e}"));
        for (index, text) in texts.iter().enumerate() {
            let want: Vec<std::ops::Range<usize>> = entry["m"]
                .get(index.to_string())
                .map(|spans| {
                    spans
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|s| {
                            bytes(text, s[0].as_u64().unwrap())..bytes(text, s[1].as_u64().unwrap())
                        })
                        .collect()
                })
                .unwrap_or_default();
            hits += want.len();
            let got = pattern.find_all(text).unwrap();
            if got != want {
                failures.push(format!(
                    "{source:?} on {text:?}: want {want:?}, got {got:?}"
                ));
            }
        }
    }
    assert!(
        hits > 1000,
        "the texts must exercise the patterns: {hits} matches"
    );
    assert!(
        failures.is_empty(),
        "{} differences, first:\n{}",
        failures.len(),
        failures
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
