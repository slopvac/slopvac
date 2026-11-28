//! Each name check in the parent module has a JSON Schema pattern that the generated schema
//! publishes. These properties hold the two to the same verdict on any string, so the check
//! that rejects a document and the pattern an editor applies cannot drift apart.
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

use super::*;

fn validator(schema: &serde_json::Value) -> Validator {
    jsonschema::validator_for(schema).expect("the pattern compiles")
}

fn by_pattern(pattern: &str) -> Validator {
    validator(&json!({"type": "string", "pattern": pattern}))
}

static PACK: LazyLock<Validator> = LazyLock::new(|| by_pattern(slopvac_model::SLUG));
static REF: LazyLock<Validator> = LazyLock::new(|| by_pattern(REF_PATTERN));
static RULE_KEY: LazyLock<Validator> = LazyLock::new(|| by_pattern(RULE_KEY_PATTERN));
static PLUGIN: LazyLock<Validator> =
    LazyLock::new(|| by_pattern(&slopvac_model::plugin_id_pattern()));
static INDEX_NAME: LazyLock<Validator> = LazyLock::new(|| by_pattern(&index_name_pattern()));
static INDEX_SOURCE: LazyLock<Validator> = LazyLock::new(|| {
    validator(&json!({
        "type": "string",
        "maxLength": INDEX_SOURCE_MAX,
        "pattern": INDEX_SOURCE_PATTERN,
    }))
});

/// Strings near the boundaries of the name forms, and arbitrary text.
fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9:._ \\-\n@]{0,10}",
        "(core|slopvac|core:x|x:core)[a-z\n]{0,2}",
        any::<String>(),
    ]
}

/// Strings that reach each branch of the source check: a valid scheme with a host part that
/// holds user-info, white space and delimiters, a refused scheme, and arbitrary text.
fn source() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "(oci|https)://[a-z0-9@ \t/?#.:\u{a0}\n-]{0,14}",
        1 => "(http|git\\+https|file)?(://)?[a-z0-9@ /?#.:-]{0,14}",
        1 => any::<String>(),
    ]
}

fn valid(validator: &Validator, s: &str) -> bool {
    validator.is_valid(&json!(s))
}

proptest! {
    #[test]
    fn the_slug_check_and_pattern_agree(s in text()) {
        prop_assert_eq!(is_pack_id(&s), valid(&PACK, &s));
    }

    #[test]
    fn the_reference_check_and_pattern_agree(s in text()) {
        prop_assert_eq!(is_ref(&s), valid(&REF, &s));
    }

    #[test]
    fn the_rule_key_check_and_pattern_agree(s in text()) {
        prop_assert_eq!(is_rule_key(&s), valid(&RULE_KEY, &s));
    }

    #[test]
    fn the_plugin_id_check_and_pattern_agree(s in text()) {
        prop_assert_eq!(is_plugin_id(&s), valid(&PLUGIN, &s));
    }

    #[test]
    fn the_index_name_check_and_pattern_agree(s in text()) {
        prop_assert_eq!(is_index_name(&s), valid(&INDEX_NAME, &s));
    }

    #[test]
    fn the_index_source_check_and_pattern_agree(s in source()) {
        prop_assert_eq!(index_source_fault(&s).is_none(), valid(&INDEX_SOURCE, &s));
    }
}

#[test]
fn an_index_source_one_character_over_the_cap_is_refused_by_both() {
    let host = "oci://h/";
    let at = format!("{host}{}", "a".repeat(INDEX_SOURCE_MAX - host.len()));
    let over = format!("{at}a");
    assert!(index_source_fault(&at).is_none() && valid(&INDEX_SOURCE, &at));
    assert!(index_source_fault(&over).is_some() && !valid(&INDEX_SOURCE, &over));
}

/// The two code points where the schema engines read `\s` differently from
/// `char::is_whitespace`. The Rust engines read `\s` as Unicode `White_Space`, so for each code
/// point the reader and the Rust schema validator take the same side: U+FEFF is accepted by
/// both, and U+0085 is refused by both. ECMA-262 takes the other side of each (it refuses
/// U+FEFF and accepts U+0085), which is why the published pattern stays pending in
/// `slopvac-schemas`. The pattern and its pending entry are unchanged.
#[rstest]
#[case::byte_order_mark(0xFEFF, true)]
#[case::next_line(0x85, false)]
fn the_reader_and_the_rust_schema_take_the_same_side_where_engines_disagree(
    #[case] code: u32,
    #[case] accepted: bool,
) {
    let point = char::from_u32(code).unwrap();
    let authority = format!("oci://h{point}x/a");
    let path = format!("https://h/a{point}b");
    for source in [authority, path] {
        assert_eq!(
            index_source_fault(&source).is_none(),
            accepted,
            "reader {source:?}"
        );
        assert_eq!(
            valid(&INDEX_SOURCE, &source),
            accepted,
            "validator {source:?}"
        );
    }
}

/// The `RelPath` pattern as the published style schema states it.
static REL_PATH: LazyLock<Validator> = LazyLock::new(|| {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/style.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let def = schema.pointer("/$defs/RelPath").unwrap();
    validator(def)
});

/// Paths near the edges of the relative-path rule: separators, drive letters, `..` segments,
/// and the code points where engines read white space or `.` differently. Line feed is left
/// out because the pattern's `.` stops at it, and `rel_path_with_a_line_feed` records that.
fn rel_path() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9./\\\\: \u{85}\u{feff}-]{0,8}",
        "(\\.\\./|a/\\.\\.|[A-Za-z]:|/|\\\\)[a-z./]{0,4}",
        any::<String>().prop_filter("a line feed ends `.` in the pattern", |s| !s.contains('\n')),
    ]
}

proptest! {
    #[test]
    fn the_relative_path_check_and_pattern_agree(s in rel_path()) {
        prop_assert_eq!(is_rel_path(&s), valid(&REL_PATH, &s));
    }
}

/// Where the published pattern and the reader differ or agree on the code points that engines
/// read differently. The pattern's `.` stops at a line feed, so a backslash after one escapes
/// the pattern's `(?!.*\\)`, and the reader refuses it. The pattern stays as published. U+0085
/// and U+FEFF are plain characters to both.
#[rstest]
#[case::line_feed_before_a_backslash("a\n\\b", false, true)]
#[case::next_line_before_a_backslash("a\u{85}\\b", false, false)]
#[case::next_line_alone("\u{85}", true, true)]
#[case::byte_order_mark_inside("a\u{feff}b", true, true)]
#[case::byte_order_mark_before_dots("\u{feff}..", true, true)]
fn rel_path_cases_where_engines_read_characters_differently(
    #[case] path: &str,
    #[case] reader: bool,
    #[case] pattern: bool,
) {
    assert_eq!(is_rel_path(path), reader, "reader");
    assert_eq!(valid(&REL_PATH, path), pattern, "pattern");
}
