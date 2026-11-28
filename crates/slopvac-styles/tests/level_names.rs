//! `Level` stays a serde enum, because a derive cannot read a constant. This test holds its
//! names equal to `slopvac_config::RULE_LEVELS`, the one list the config reader and both
//! schemas use. A variant that is added, removed or renamed on either side fails here. The
//! order of the list is not part of the contract.
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::collections::BTreeSet;

use serde::Deserialize;
use serde::de::IntoDeserializer;
use serde::de::value::{Error, StrDeserializer};
use slopvac_config::RULE_LEVELS;
use slopvac_styles::Level;

/// Every variant of `Level`. The match has no wildcard arm, so adding a variant stops this
/// file compiling until the list and `RULE_LEVELS` take it in.
const VARIANTS: [Level; 4] = [Level::Error, Level::Warning, Level::Suggestion, Level::Off];

fn variant_is_listed(level: Level) -> bool {
    match level {
        Level::Error | Level::Warning | Level::Suggestion | Level::Off => VARIANTS.contains(&level),
    }
}

fn parse(name: &str) -> Option<Level> {
    let name: StrDeserializer<'_, Error> = name.into_deserializer();
    Level::deserialize(name).ok()
}

#[test]
fn the_level_names_equal_the_shared_list() {
    assert!(VARIANTS.iter().all(|level| variant_is_listed(*level)));
    let parsed: Vec<Option<Level>> = RULE_LEVELS.iter().map(|name| parse(name)).collect();
    assert!(
        parsed.iter().all(Option::is_some),
        "a shared name `Level` refuses: {parsed:?}"
    );
    let distinct: BTreeSet<String> = parsed.iter().map(|l| format!("{:?}", l.unwrap())).collect();
    let expected: BTreeSet<String> = VARIANTS.iter().map(|l| format!("{l:?}")).collect();
    assert_eq!(
        RULE_LEVELS.len(),
        VARIANTS.len(),
        "a name is listed twice or missing"
    );
    assert_eq!(distinct, expected);
}
