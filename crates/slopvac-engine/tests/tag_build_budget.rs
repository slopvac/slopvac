//! A document cannot make the engine build a tag index of unbounded size: the engine counts
//! the tags against a ceiling that follows the document's size and ends the run before it
//! builds anything.
//!
//! The tests count the elements the index structures are built from, not the time.
#![cfg(unix)]
#![allow(missing_docs)]

mod common;

use serde_json::json;
use slopvac_context::take_build_elements;
use slopvac_engine::{Engine, EngineError, Input};

fn engine() -> Engine {
    common::engine(&[common::tokens("r", "leverage", &json!({}))])
}

/// A paragraph of `sentences` copies of `sentence`.
fn paragraph(sentence: &str, sentences: usize) -> String {
    format!("{}\n", vec![sentence; sentences].join(" "))
}

#[test]
fn a_paragraph_of_very_short_requirements_is_not_checked_and_builds_no_index() {
    // 250000 sentences of 8 bytes: one tag for every 9 bytes, over the one for every 16 the
    // ceiling allows. The ceiling for 2.25 MB is 206 thousand tags.
    let source = paragraph("It must.", 250_000);
    let _ = take_build_elements();
    let result = engine().lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::TagBuildBudget {
            limit: source.len() / 16 + 65_536
        })
    );
    assert_eq!(take_build_elements(), 0, "an index was built");
}

#[test]
fn a_normal_document_is_checked_and_builds_its_index_once() {
    // 2000 sentences of 68 bytes, each a requirement: far under the ceiling of 66 thousand.
    let sentence = "The service must reject every unsigned request before it parses one.";
    let source = paragraph(sentence, 2_000);
    let _ = take_build_elements();
    let doc = engine()
        .lint(&Input::markdown("a.md", &source))
        .expect("a normal document is checked");
    assert_eq!(doc.findings.len(), 0);
    // Each sentence is a requirement and an enforced invariant: 4000 tags. One structure over
    // them counts 4000; a second index over the same tags would count 8000.
    assert_eq!(take_build_elements(), 4_000);
}
