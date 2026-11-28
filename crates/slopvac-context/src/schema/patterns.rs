//! Each name check that `slopvac-config` exports has a JSON Schema pattern that the generated genre
//! schema publishes: the context tag and the genre id share the model's slug, and the style and
//! genre references and the rule and category names share the config definitions. These
//! properties hold the check and the pattern to the same verdict on any string, so the check
//! that rejects a genre file and the pattern an editor applies cannot drift apart.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a panic is a failed test"
)]

use std::sync::LazyLock;

use jsonschema::Validator;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::json;

use slopvac_config::{is_name, is_ref, is_rule_key};

fn by_pattern(pattern: &str) -> Validator {
    jsonschema::validator_for(&json!({"type": "string", "pattern": pattern}))
        .expect("the pattern compiles")
}

/// The pattern of one definition of the config schema.
fn config_pattern(name: &str) -> String {
    slopvac_config::config_schema()["$defs"][name]["pattern"]
        .as_str()
        .expect("the definition has a pattern")
        .to_owned()
}

static NAME: LazyLock<Validator> = LazyLock::new(|| by_pattern(slopvac_model::SLUG));
static REF: LazyLock<Validator> = LazyLock::new(|| by_pattern(&config_pattern("Ref")));
static RULE_KEY: LazyLock<Validator> = LazyLock::new(|| by_pattern(&config_pattern("RuleKey")));

/// Strings near the boundaries of the name forms, and arbitrary text.
fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9:._ \\-\n]{0,10}",
        "[a-z]{1,3}[:\n\u{feff}\u{85}\u{2028}]?[a-z]{0,2}[\n\u{feff}\u{85}]?",
        any::<String>(),
    ]
}

fn valid(validator: &Validator, s: &str) -> bool {
    validator.is_valid(&json!(s))
}

proptest! {
    #[test]
    fn the_name_check_and_pattern_agree(s in text()) {
        prop_assert_eq!(is_name(&s), valid(&NAME, &s));
    }

    #[test]
    fn the_reference_check_and_pattern_agree(s in text()) {
        prop_assert_eq!(is_ref(&s), valid(&REF, &s));
    }

    #[test]
    fn the_rule_key_check_and_pattern_agree(s in text()) {
        prop_assert_eq!(is_rule_key(&s), valid(&RULE_KEY, &s));
    }
}

/// Characters that `\s` and `\S` treat differently across engines: the byte order mark, the
/// next-line control, the line and paragraph separators and a trailing line feed. A name never
/// holds one, so the check and the pattern both refuse each.
#[rstest]
#[case::trailing_line_feed("core\n")]
#[case::leading_line_feed("\ncore")]
#[case::trailing_bom("core\u{feff}")]
#[case::leading_bom("\u{feff}core")]
#[case::inner_bom("co\u{feff}re")]
#[case::trailing_next_line("core\u{85}")]
#[case::leading_next_line("\u{85}core")]
#[case::inner_next_line("co\u{85}re")]
#[case::trailing_line_separator("core\u{2028}")]
#[case::bom_after_the_colon("a:\u{feff}b")]
#[case::next_line_before_the_colon("a\u{85}:b")]
fn a_control_or_separator_makes_every_name_form_invalid(#[case] s: &str) {
    for (what, by_check, by_pattern) in [
        ("name", is_name(s), valid(&NAME, s)),
        ("ref", is_ref(s), valid(&REF, s)),
        ("rule key", is_rule_key(s), valid(&RULE_KEY, s)),
    ] {
        assert!(!by_check, "the {what} check accepts {s:?}");
        assert!(!by_pattern, "the {what} pattern accepts {s:?}");
    }
}

#[rstest]
#[case::bare("core", true, true, true)]
#[case::qualified("core:article", false, true, true)]
#[case::rule_key_with_dots("core:no-hedge.v2", false, false, true)]
#[case::upper_case_name("Core", false, false, true)]
#[case::empty("", false, false, false)]
fn each_form_accepts_what_it_names(
    #[case] s: &str,
    #[case] name: bool,
    #[case] reference: bool,
    #[case] rule_key: bool,
) {
    assert_eq!((is_name(s), valid(&NAME, s)), (name, name), "name {s:?}");
    assert_eq!(
        (is_ref(s), valid(&REF, s)),
        (reference, reference),
        "ref {s:?}"
    );
    assert_eq!(
        (is_rule_key(s), valid(&RULE_KEY, s)),
        (rule_key, rule_key),
        "rule key {s:?}"
    );
}
