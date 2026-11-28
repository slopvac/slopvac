//! The level names have one declaration, `Level`. The list of names, the reader and the
//! generated schema all follow it.
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use slopvac_config::{Level, RULE_LEVELS};

fn parse(name: &str) -> Result<Level, String> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).map_err(|e| e.to_string())
}

#[test]
fn the_name_list_is_the_spelling_of_every_level_in_order() {
    let spelled: Vec<&str> = Level::ALL.iter().map(|level| level.as_str()).collect();
    assert_eq!(RULE_LEVELS, spelled.as_slice());
}

#[test]
fn every_name_parses_back_to_its_own_level() {
    for level in Level::ALL {
        assert_eq!(parse(level.as_str()), Ok(level), "{}", level.as_str());
    }
}

#[test]
fn a_name_outside_the_list_is_refused_with_the_list() {
    let message = parse("hint").unwrap_err();
    for name in RULE_LEVELS {
        assert!(message.contains(name), "`{name}` missing from: {message}");
    }
}

#[test]
fn no_two_levels_share_a_spelling() {
    let mut names: Vec<&str> = RULE_LEVELS.to_vec();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Level::ALL.len());
}
