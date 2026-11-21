#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]
//! The vocabulary sweep indexes the source once per document, not once per raw HTML block.
//! The work counter is the number of `SourceIndex` values built. It is deterministic, and
//! it lives in its own test binary so no other test builds indexes at the same time.

mod common;

use common::{category, rule, ruleset, settings};
use serde_json::json;
use slopvac_engine::{Engine, Input};
use slopvac_terms::Blocklist;

fn builds_for(blocks: usize) -> (usize, usize) {
    let rules = [rule(
        "word-outside-controlled-vocabulary",
        &json!({"kind": "vocabulary", "check": "word-outside-controlled-vocabulary"}),
        &json!({}),
    )];
    let list = Blocklist::parse(
        "b.toml",
        "toml",
        "schema_version = 3\n[[entries]]\nword = \"request\"\npos = \"verb\"\nreason = \"r\"\n",
    )
    .unwrap();
    let engine = Engine::new(
        vec![ruleset(vec![category("ste-words", &rules)])],
        settings(),
    )
    .unwrap()
    .with_blocklist(&list)
    .unwrap();
    let source = "> <p>Please request\n> data</p>\n\ntext\n\n".repeat(blocks);
    let before = slopvac_model::indexes_built();
    let doc = engine.lint(&Input::markdown("a.md", &source)).unwrap();
    (doc.findings.len(), slopvac_model::indexes_built() - before)
}

#[test]
fn indexes_built_do_not_grow_with_the_number_of_html_blocks() {
    let (few_findings, few) = builds_for(2);
    let (many_findings, many) = builds_for(200);
    assert_eq!(
        (few_findings, many_findings),
        (2, 200),
        "every block is swept"
    );
    assert_eq!(
        many, few,
        "a document builds its indexes once, whatever its blocks"
    );
}
