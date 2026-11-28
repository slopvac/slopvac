//! Values that a genre file shares with the config schema. `RuleLevel` names are the ones the
//! config schema publishes as `RuleLevel`, which `slopvac-config` keeps in `RULE_LEVELS`, and
//! the budget sentinel is its `BUDGET_NONE`. The generated genre schema takes both from the
//! config schema, so a reader variant that drifts from them fails here and not in a user's file.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::collections::BTreeSet;

use serde_json::json;
use slopvac_config::RULE_LEVELS;
use slopvac_context::{Budget, NoBudget, RuleLevel};

#[test]
fn every_published_level_name_reads_as_a_level() {
    for name in RULE_LEVELS {
        let level = serde_json::from_value::<RuleLevel>(json!(name));
        assert!(level.is_ok(), "`{name}` does not read as a RuleLevel");
    }
}

#[test]
fn the_published_level_names_reach_every_variant_once() {
    let reached: BTreeSet<String> = RULE_LEVELS
        .iter()
        .map(|name| {
            format!(
                "{:?}",
                serde_json::from_value::<RuleLevel>(json!(name)).unwrap()
            )
        })
        .collect();
    let variants: BTreeSet<String> = ["Error", "Warning", "Suggestion", "Off"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(reached, variants);
    assert_eq!(RULE_LEVELS.len(), variants.len());
}

#[test]
fn the_budget_sentinel_that_config_publishes_reads_as_no_budget() {
    let budget: Budget = serde_json::from_value(json!(slopvac_config::BUDGET_NONE)).unwrap();
    assert_eq!(budget, Budget::None(NoBudget::None));
}
