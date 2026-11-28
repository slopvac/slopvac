//! `Level` stays a serde enum, because a derive cannot read a constant. This test holds its
//! names equal to `slopvac_config::RULE_LEVELS`, the one list the config reader and both
//! schemas use. A variant that is added, removed, renamed or moved on either side fails here.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use serde::Deserialize;
use serde::de::IntoDeserializer;
use serde::de::value::{Error, StrDeserializer};
use slopvac_config::RULE_LEVELS;
use slopvac_styles::Level;

/// The variants of `Level` in declaration order. The match has no wildcard arm, so adding a
/// variant stops this file compiling until the list and `RULE_LEVELS` take it in.
const DECLARED: [Level; 4] = [Level::Error, Level::Warning, Level::Suggestion, Level::Off];

fn position(level: Level) -> usize {
    match level {
        Level::Error => 0,
        Level::Warning => 1,
        Level::Suggestion => 2,
        Level::Off => 3,
    }
}

#[test]
fn the_level_names_equal_the_shared_list_in_order() {
    let parsed: Vec<Option<Level>> = RULE_LEVELS
        .iter()
        .map(|name| {
            let name: StrDeserializer<'_, Error> = (*name).into_deserializer();
            Level::deserialize(name).ok()
        })
        .collect();
    let expected: Vec<Option<Level>> = DECLARED.iter().copied().map(Some).collect();
    assert_eq!(parsed, expected);
    assert!(DECLARED.iter().enumerate().all(|(i, l)| position(*l) == i));
}
