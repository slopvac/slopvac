//! Values that a genre file shares with the config schema. `RuleLevel` names are the ones the
//! config schema publishes as `RuleLevel`, which `slopvac-config` keeps in `RULE_LEVELS`, and
//! the budget sentinel is its `BUDGET_NONE`. The generated genre schema takes both from the
//! config schema, so a reader variant that drifts from them fails here and not in a user's file.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use serde_json::json;
use slopvac_config::RULE_LEVELS;
use slopvac_context::{Budget, NoBudget, RuleLevel};

/// Every variant of `RuleLevel`, listed through a match that stops compiling when one is added.
fn every_level() -> [RuleLevel; 4] {
    let all = [
        RuleLevel::Error,
        RuleLevel::Warning,
        RuleLevel::Suggestion,
        RuleLevel::Off,
    ];
    for level in all {
        match level {
            RuleLevel::Error | RuleLevel::Warning | RuleLevel::Suggestion | RuleLevel::Off => {}
        }
    }
    all
}

#[test]
fn the_levels_the_reader_reads_are_exactly_the_published_level_names() {
    let mut read: Vec<String> = Vec::new();
    for name in RULE_LEVELS {
        let level = serde_json::from_value::<RuleLevel>(json!(name));
        assert!(level.is_ok(), "`{name}` does not read as a RuleLevel");
        read.push(format!("{:?}", level.unwrap()).to_lowercase());
    }
    let mut variants: Vec<String> = every_level()
        .iter()
        .map(|level| format!("{level:?}").to_lowercase())
        .collect();
    read.sort();
    variants.sort();
    assert_eq!(
        read, variants,
        "RULE_LEVELS and the variants of RuleLevel differ"
    );
}

#[test]
fn the_budget_sentinel_that_config_publishes_reads_as_no_budget() {
    let budget: Budget = serde_json::from_value(json!(slopvac_config::BUDGET_NONE)).unwrap();
    assert_eq!(budget, Budget::None(NoBudget::None));
}
