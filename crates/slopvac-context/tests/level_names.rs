//! `RuleLevel` is the level a genre sets for a category or a rule. Its names are the ones the
//! config schema publishes as `RuleLevel`, which `slopvac-config` keeps in `RULE_LEVELS`. The
//! reader and the generated schema read that one list, so a variant that drifts from it fails
//! here and not in a user's genre file.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::collections::BTreeSet;

use serde_json::json;
use slopvac_config::RULE_LEVELS;
use slopvac_context::RuleLevel;

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
