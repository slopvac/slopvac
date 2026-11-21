//! `CategoryDials::budget_cleared`: a `"none"` budget removes the budget that the rule data
//! gives for the profile, from the report and from the gate.
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

use common::{engine_with, settings, tokens};
use serde_json::json;
use slopvac_engine::{CategoryDials, Input};
use slopvac_model::{CategoryId, Document};

/// A category whose rule data sets a budget of 0.1 findings per 100 words for `normal`, over
/// a document of 100 words with one finding: 1.0 per 100 words, over budget.
fn lint(clear: bool) -> Document {
    let category = json!({
        "schema_version": 3, "id": "cat", "title": "t", "description": "d",
        "max_per_100_words": {"normal": 0.1},
        "rules": [tokens("hit", "leverage", &json!({"severity": "warning"}))],
    });
    let mut s = settings();
    if clear {
        s.categories.insert(
            CategoryId::new("cat").unwrap(),
            CategoryDials::default().with_budget_cleared(true),
        );
    }
    let filler = "word ".repeat(98);
    let text = format!("We leverage it. {filler}\n");
    let engine = engine_with(&category, s);
    engine.lint(&Input::markdown("doc.md", &text)).unwrap()
}

#[test]
fn the_rule_data_budget_applies_when_the_config_does_not_clear_it() {
    let doc = lint(false);
    let c = &doc.categories[0];
    assert_eq!(c.budget.map(slopvac_model::Density::get), Some(0.1));
    assert!(c.over_budget);
    assert!(!doc.gate.is_passed(), "{:?}", doc.gate.reasons());
}

#[test]
fn a_cleared_budget_is_absent_from_the_report_and_cannot_fail_the_gate() {
    let doc = lint(true);
    let c = &doc.categories[0];
    assert_eq!(c.budget, None);
    assert!(!c.over_budget);
    assert!(
        !doc.gate.reasons().iter().any(|r| r.contains("budget")),
        "{:?}",
        doc.gate.reasons()
    );
}
