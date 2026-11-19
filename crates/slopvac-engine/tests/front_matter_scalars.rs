//! Front matter values that are not strings reach the context taggers as text. A tagger that
//! reads a key sees `true`, `2024` or `1.5` for the scalar as written, and nothing for a
//! null, a sequence or a table. The last occurrence of a repeated key decides.
//!
//! Each case lints a real document through `Engine::lint`. A configurable tagger sets the
//! `generated` context when the key `flag` matches its pattern, and a rule that skips
//! `generated` text goes silent exactly when the tagger fired.

#![allow(missing_docs)]

mod common;

use common::{engine, rule, tokens};
use rstest::rstest;
use serde_json::json;
use slopvac_context::{TagRegistry, TaggerSpec};
use slopvac_engine::Input;

const BODY: &str = "We leverage the cache.\n";

/// Whether the rule stayed silent, so the tagger fired, on `front` followed by the body.
fn tagged(front: &str, pattern: &str) -> bool {
    let spec = format!(
        "schema_version: 3\nid: flag-gate\ntag: generated\nscope: document\nwhen:\n  \
         front_matter:\n    key: flag\n    matches: '{pattern}'\n"
    );
    let mut registry = TagRegistry::new();
    registry
        .register(Box::new(
            TaggerSpec::from_yaml(&spec)
                .unwrap()
                .compile()
                .unwrap(),
        ))
        .unwrap();
    let gated = rule(
        "gated",
        &tokens("gated", "leverage", &json!({}))["match"],
        &json!({"contexts": {"skip": ["generated"]}}),
    );
    let engine = engine(&[gated]).with_taggers(registry);
    let doc = engine
        .lint(&Input::markdown("a.md", &format!("{front}{BODY}")))
        .unwrap();
    doc.findings.is_empty()
}

#[rstest]
#[case::yaml_bool("---\nflag: true\n---\n", "^true$", true)]
#[case::yaml_bool_false("---\nflag: false\n---\n", "^true$", false)]
#[case::yaml_integer("---\nflag: 2024\n---\n", "^2024$", true)]
#[case::yaml_float("---\nflag: 1.5\n---\n", "^1.5$", true)]
#[case::yaml_quoted_number("---\nflag: \"2024\"\n---\n", "^2024$", true)]
#[case::yaml_string("---\nflag: maybe\n---\n", "^maybe$", true)]
#[case::yaml_null("---\nflag:\n---\n", ".*", false)]
#[case::yaml_sequence("---\nflag: [a, b]\n---\n", ".*", false)]
#[case::toml_bool("+++\nflag = true\n+++\n", "^true$", true)]
#[case::toml_integer("+++\nflag = 7\n+++\n", "^7$", true)]
#[case::toml_date("+++\nflag = 2024-05-01\n+++\n", "^2024-05-01$", true)]
#[case::toml_datetime(
    "+++\nflag = 2024-05-01T07:32:00Z\n+++\n",
    "^2024-05-01T07:32:00Z$",
    true
)]
fn a_scalar_reaches_the_tagger_as_text(
    #[case] front: &str,
    #[case] pattern: &str,
    #[case] fires: bool,
) {
    assert_eq!(tagged(front, pattern), fires);
}
