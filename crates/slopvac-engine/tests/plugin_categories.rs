//! Two plugins may share a bare category id. The engine scores each plugin's category on
//! its own weight and budget, and the report names it `<plugin>:<category>`.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs)]

mod common;

use std::path::PathBuf;

use common::{engine, settings, tokens};
use serde_json::json;
use slopvac_engine::{Engine, Input};
use slopvac_model::{Document, PackId};
use slopvac_rules::{CompileOptions, RuleSet};

/// A rule set of plugin `plugin` with one category `style`: weight, a budget for `normal`, and
/// one tokens rule `hit` for `token` at `severity`.
fn plugin_set(plugin: &str, weight: f64, budget: f64, token: &str, severity: &str) -> RuleSet {
    let category = json!({
        "schema_version": 3, "id": "style", "title": "t", "description": "d",
        "weight": weight,
        "max_per_100_words": {"normal": budget},
        "rules": [tokens("hit", token, &json!({"severity": severity}))],
    });
    RuleSet::from_sources(
        &PackId::new(plugin).unwrap(),
        vec![(PathBuf::from("style.json"), category.to_string())],
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

/// A document of exactly 100 words that hits each plugin's rule once.
fn lint_both() -> Document {
    // alpha: weight 1, budget 2.0, one warning.
    // beta: weight 3, budget 0.5, one error.
    let sets = vec![
        plugin_set("alpha", 1.0, 2.0, "leverage", "warning"),
        plugin_set("beta", 3.0, 0.5, "synergy", "error"),
    ];
    let engine = Engine::new(sets, settings()).unwrap();
    let text = format!("leverage synergy {}\n", "word ".repeat(98));
    engine.lint(&Input::markdown("doc.md", &text)).unwrap()
}

#[test]
fn two_plugins_with_one_bare_category_id_score_as_two_categories() {
    let doc = lint_both();
    let names: Vec<&str> = doc.categories.iter().map(|c| c.category.as_str()).collect();
    assert_eq!(names, ["alpha:style", "beta:style"]);

    // alpha: one warning weighs 0.5 in 100 words, so 0.5 per 100 against a budget of 2.0:
    // 100 - 30 * (0.5 / 2.0) = 92.5, inside the budget.
    let alpha = &doc.categories[0];
    assert_eq!(alpha.findings, 1);
    assert_eq!(alpha.budget.map(slopvac_model::Density::get), Some(2.0));
    assert_eq!(alpha.score.get(), 92.5);
    assert!(!alpha.over_budget);

    // beta: one error weighs 1.0 per 100 against a budget of 0.5: over budget, and
    // 70 * (1 - (1.0 - 0.5) / (0.5 * 3)) = 46.67, shown as 46.7.
    let beta = &doc.categories[1];
    assert_eq!(beta.findings, 1);
    assert_eq!(beta.budget.map(slopvac_model::Density::get), Some(0.5));
    assert_eq!(beta.score.get(), 46.7);
    assert!(beta.over_budget);
    assert_eq!(
        doc.gate.reasons(),
        ["beta:style at 1.00 severity-weighted findings per 100 words, budget 0.5"]
    );
}

#[test]
fn a_finding_names_its_category_by_plugin_unless_the_plugin_is_core() {
    let doc = lint_both();
    let by_rule: Vec<(&str, &str)> = doc
        .findings
        .iter()
        .map(|f| (f.rule_id.as_str(), f.category.as_str()))
        .collect();
    assert_eq!(
        by_rule,
        [
            ("alpha:style.hit", "alpha:style"),
            ("beta:style.hit", "beta:style")
        ]
    );

    // The same rule in the `core` plugin keeps the bare category id.
    let core = engine(&[tokens("hit", "leverage", &json!({}))]);
    let text = format!("leverage {}\n", "word ".repeat(99));
    let doc = core.lint(&Input::markdown("doc.md", &text)).unwrap();
    assert_eq!(doc.findings[0].category.as_str(), "cat");
    assert_eq!(doc.categories[0].category.as_str(), "cat");
}
