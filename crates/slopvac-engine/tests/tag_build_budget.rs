//! A document cannot make the engine build a tag index of unbounded size: the engine charges
//! each tag against a ceiling that follows the document's size before it builds the tag, and
//! ends the run before it builds any index.
//!
//! The tests count the tags built and the elements the index structures are built from, not
//! the time.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![cfg(unix)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

mod common;

use common::{wrapped_rst, wrapped_table};
use serde_json::json;
use slopvac_context::{TagRegistry, TaggerSpec, take_build_elements, take_materialised_tags};
use slopvac_engine::{Engine, EngineError, Input};

/// The ceiling in `docs/contract/policies.md`: one tag for every 64 bytes of the document,
/// plus 65,536.
fn ceiling(source_len: usize) -> usize {
    source_len / 64 + 65_536
}

fn engine() -> Engine {
    common::engine(&[common::tokens("r", "leverage", &json!({}))])
}

/// A paragraph of `sentences` copies of `sentence`.
fn paragraph(sentence: &str, sentences: usize) -> String {
    format!("{}\n", vec![sentence; sentences].join(" "))
}

/// `kinds` taggers, each tagging a table cell that holds `Must` with a kind of its own.
fn taggers(kinds: usize) -> TagRegistry {
    let mut registry = TagRegistry::new();
    for i in 0..kinds {
        let yaml = format!(
            "schema_version: 3\nid: t{i}\ntag: k{i}\nunit: table-cell\nmatch:\n  - pattern: 'Must'\n"
        );
        let tagger = TaggerSpec::from_yaml(&yaml).unwrap().compile().unwrap();
        registry.register(Box::new(tagger)).unwrap();
    }
    registry
}

#[test]
fn a_paragraph_of_very_short_requirements_is_not_checked_and_builds_no_tag_past_the_ceiling() {
    // 150000 sentences of 8 bytes: one tag for every 9 bytes, over the one for every 64 the
    // ceiling allows.
    let source = paragraph("It must.", 150_000);
    let limit = ceiling(source.len());
    let _ = (take_build_elements(), take_materialised_tags());
    let result = engine().lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::TagBuildBudget { limit })
    );
    assert_eq!(take_build_elements(), 0, "an index was built");
    // The requirement tagger charges each tag before it builds it, so it builds the limit
    // and no more.
    assert_eq!(take_materialised_tags(), limit);
}

#[test]
fn tags_that_fit_the_ceiling_but_split_into_pieces_past_it_are_not_checked() {
    // 1500 kinds of tag on one cell of 60 lines: 1500 tags, far under the ceiling, that split
    // into 90000 pieces, over the ceiling of about 65,600.
    let (kinds, lines) = (1_500, 60);
    let source = wrapped_table(lines);
    let limit = ceiling(source.len());
    let engine = engine()
        .with_rst(wrapped_rst(lines))
        .with_taggers(taggers(kinds));
    let _ = (take_build_elements(), take_materialised_tags());
    let result = engine.lint(&Input::rst("t.rst", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::TagBuildBudget { limit })
    );
    assert_eq!(take_build_elements(), 0, "an index was built");
    // The 1500 tags, then pieces until the next one would pass the ceiling.
    assert_eq!(take_materialised_tags(), kinds + limit);
}

#[test]
fn a_normal_document_is_checked_and_builds_its_index_once() {
    // 2000 sentences of 68 bytes, each a requirement: far under the ceiling of 100 thousand.
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

#[test]
fn a_paragraph_of_one_character_sentences_is_not_checked_before_its_sentences_are_all_built() {
    // 300000 sentences of 4 bytes, none of which any tagger tags. One sentence for every 8
    // bytes, plus 65,536, allows 150,000 + 65,536 = 215,536 of them, so the split stops there
    // and the run ends before it holds the rest.
    let source = paragraph("Go.", 300_000);
    let limit = source.len() / 8 + 65_536;
    let result = engine().lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::TagBuildBudget { limit })
    );
}
