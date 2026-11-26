//! The generated lock `source` pattern refuses what the lock reader refuses as unsafe.
//!
//! The reader rejects any character that `slopvac_sanitize::is_unsafe` flags. The schema's
//! pattern is built from `slopvac_sanitize::unsafe_ranges`. This test ties the two, so a
//! change to either side that lets them diverge fails here. It reads the generated schema,
//! and `slopvac-schemas check` (the `schemas-check` CI step) keeps the committed file equal to it.

use std::collections::BTreeSet;

use jsonschema::Validator;
use serde_json::{Value, json};
use slopvac_plugins::lock::Source;
use slopvac_schemas::generate;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[expect(
    clippy::unwrap_used,
    reason = "test helper: a failure to build the validator is a failed test"
)]
fn validator() -> Validator {
    let generated = generate().unwrap();
    let file = generated
        .iter()
        .find(|g| g.file == "plugins-lock.schema.json")
        .unwrap();
    let schema: Value = serde_json::from_str(&file.text).unwrap();
    let pattern = schema
        .pointer("/$defs/Locked/properties/source/pattern")
        .and_then(Value::as_str)
        .unwrap();
    jsonschema::draft202012::new(&json!({"type": "string", "pattern": pattern})).unwrap()
}

/// Low bound, high bound and one middle code point of every unsafe range.
#[expect(
    clippy::unwrap_used,
    reason = "test helper: every table entry is a scalar value, or the test fails"
)]
fn samples() -> BTreeSet<char> {
    let mut points = BTreeSet::new();
    for &(low, high) in slopvac_sanitize::unsafe_ranges() {
        for code in [low, low + (high - low) / 2, high] {
            points.insert(char::from_u32(code).unwrap());
        }
    }
    points
}

/// One source per branch of the pattern, with `c` where that branch reads freely.
fn shapes(c: char) -> [String; 6] {
    [
        format!("pkg/a{c}b"),
        format!("https://h/a{c}b"),
        format!("oci://h{c}x/p"),
        format!("https://h{c}x/p"),
        format!("git+https://h/r{c}@{COMMIT}"),
        format!("git+https://h{c}x/r@{COMMIT}"),
    ]
}

#[test]
fn the_sweep_covers_the_whole_table() {
    // A guard against an empty table: the sweep would pass without checking anything.
    assert!(slopvac_sanitize::unsafe_ranges().len() >= 40);
    assert!(samples().contains(&'\u{FEFF}'));
}

#[test]
fn schema_and_reader_both_refuse_every_unsafe_code_point() {
    let validator = validator();
    let mut diverged = Vec::new();
    for c in samples() {
        for source in shapes(c) {
            let reader_refuses = source.parse::<Source>().is_err();
            let schema_refuses = !validator.is_valid(&json!(source));
            if !(reader_refuses && schema_refuses) {
                diverged.push(format!(
                    "U+{:04X} in {source:?}: reader refuses {reader_refuses}, schema refuses {schema_refuses}",
                    u32::from(c)
                ));
            }
        }
    }
    assert!(diverged.is_empty(), "{diverged:#?}");
}

#[test]
fn schema_and_reader_both_accept_a_safe_control() {
    let validator = validator();
    for source in shapes('x') {
        assert!(
            source.parse::<Source>().is_ok(),
            "reader refuses {source:?}"
        );
        assert!(
            validator.is_valid(&json!(source)),
            "schema refuses {source:?}"
        );
    }
}
