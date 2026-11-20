//! The Rust types agree with the published rule and category schemas, judged by the
//! conformance corpus (`schemas/v3/conformance`):
//!
//! * the published schema accepts every valid instance and rejects every invalid one;
//! * the schema generated from the Rust types does the same;
//! * `Deserialize` accepts every valid instance, and rejects each invalid one for the
//!   reason the instance names.

#![allow(missing_docs, clippy::single_range_in_vec_init)]

mod common;

use common::{file_name, generated, instances, load, published, schemas};
use serde_json::{Value, json};
use slopvac_rules::{Category, Rule, RuleFile, category_schema, rule_file_schema, rule_schema};

/// Why each invalid instance must fail: a fragment of the `Deserialize` error, worked out
/// from the defect each file names in `cases.toml`.
const WHY_RULE: &[(&str, &str)] = &[
    ("bad-comparison.json", "unknown variant `eq`"),
    (
        "context-tag-trailing-newline.json",
        "invalid context tag \"code\\n\"",
    ),
    (
        "exception-name-trailing-newline.json",
        "invalid name: \"quoted\\n\"",
    ),
    (
        "genre-trailing-newline.json",
        "invalid genre id \"reference\\n\"",
    ),
    ("case-without-assertion.json", "a case needs"),
    ("empty-contexts.json", "contexts must set"),
    ("exception-bare-string.json", "invalid type: string"),
    ("id-trailing-newline.json", "invalid rule id"),
    ("judgement-error-severity.json", "must be `suggestion`"),
    ("judgement-with-exceptions.json", "no exceptions"),
    ("kind-payload-mismatch.json", "missing field `tokens`"),
    ("metric-trailing-newline.json", "invalid metric name"),
    ("missing-citation.json", "missing field `citation`"),
    ("no-schema-version.json", "missing field `schema_version`"),
    ("pattern-long-unicode-escape.json", "invalid PCRE2 pattern"),
    ("pattern-unicode-escape.json", "invalid PCRE2 pattern"),
    (
        "placeholder-name-trailing-newline.json",
        "invalid placeholder name",
    ),
    ("placeholder-unknown-source.json", "unknown variant `env`"),
    ("qualified-id-in-rule.json", "invalid rule id"),
    ("severity-off.json", "unknown variant `off`"),
    ("ste-ref-trailing-newline.json", "invalid STE reference"),
    ("substitution-unicode-escape.json", "invalid PCRE2 pattern"),
    ("tiers-removed.json", "unknown field `tiers`"),
    ("top-level-kind.json", "unknown field `kind`"),
    ("unknown-block.json", "unknown variant `footer`"),
    ("unknown-key.json", "unknown field `colour`"),
    ("unknown-kind.json", "unknown variant `fuzzy`"),
    ("unknown-scope.json", "unknown variant `page`"),
];

const WHY_CATEGORY: &[(&str, &str)] = &[
    ("bad-rule.json", "unknown field `tiers`"),
    (
        "genre-trailing-newline.json",
        "invalid genre id \"reference\\n\"",
    ),
    ("id-trailing-newline.json", "invalid category id"),
    (
        "profile-trailing-newline.json",
        "invalid profile name: \"strict\\n\"",
    ),
    ("negative-weight.json", "invalid weight"),
    ("no-schema-version.json", "missing field `schema_version`"),
    ("profile-budget-negative.json", "invalid density"),
    ("upper-id.json", "invalid category id"),
];

fn rule_dir(sub: &str) -> std::path::PathBuf {
    schemas().join("conformance/rule").join(sub)
}

fn category_dir(sub: &str) -> std::path::PathBuf {
    schemas().join("conformance/category").join(sub)
}

/// `Rule--embedded.json` is the embedded shape; every other rule instance is a standalone
/// file.
fn is_embedded(name: &str) -> bool {
    name.starts_with("Rule--")
}

fn check(
    validator: &jsonschema::Validator,
    dir: &std::path::Path,
    want_valid: bool,
    skip_embedded: Option<bool>,
) -> usize {
    let mut n = 0;
    for path in instances(dir) {
        let name = file_name(&path);
        if skip_embedded.is_some_and(|embedded| is_embedded(&name) != embedded) {
            continue;
        }
        assert_eq!(
            validator.is_valid(&load(&path)),
            want_valid,
            "{} (valid = {want_valid})",
            path.display()
        );
        n += 1;
    }
    n
}

#[test]
fn the_corpus_covers_every_listed_defect() {
    let listed = |t: &[(&str, &str)]| t.len();
    assert_eq!(instances(&rule_dir("invalid")).len(), listed(WHY_RULE));
    assert_eq!(
        instances(&category_dir("invalid")).len(),
        listed(WHY_CATEGORY)
    );
}

#[test]
fn the_published_schemas_judge_the_corpus() {
    let standalone = published("rule.schema.json", None);
    let embedded = published("rule.schema.json", Some("Rule"));
    let category = published("category.schema.json", None);
    check(&standalone, &rule_dir("valid"), true, Some(false));
    check(&embedded, &rule_dir("valid"), true, Some(true));
    check(&standalone, &rule_dir("invalid"), false, None);
    check(&category, &category_dir("valid"), true, None);
    check(&category, &category_dir("invalid"), false, None);
}

#[test]
fn the_generated_schemas_judge_the_corpus_like_the_published_ones() {
    let standalone = generated(&rule_file_schema());
    let embedded = generated(&rule_schema());
    let category = generated(&category_schema());
    let ran = [
        check(&standalone, &rule_dir("valid"), true, Some(false)),
        check(&embedded, &rule_dir("valid"), true, Some(true)),
        check(&standalone, &rule_dir("invalid"), false, None),
        check(&category, &category_dir("valid"), true, None),
        check(&category, &category_dir("invalid"), false, None),
    ];
    assert!(ran.iter().all(|n| *n > 0), "{ran:?}");
}

#[test]
fn every_valid_rule_instance_loads() {
    for path in instances(&rule_dir("valid")) {
        let name = file_name(&path);
        let value = load(&path);
        let loaded = if is_embedded(&name) {
            serde_json::from_value::<Rule>(value).map(|_| ())
        } else {
            serde_json::from_value::<RuleFile>(value).map(|_| ())
        };
        assert!(loaded.is_ok(), "{name}: {loaded:?}");
    }
}

#[test]
fn every_valid_category_instance_loads() {
    for path in instances(&category_dir("valid")) {
        let loaded = serde_json::from_value::<Category>(load(&path));
        assert!(loaded.is_ok(), "{}: {loaded:?}", file_name(&path));
    }
}

#[test]
fn every_invalid_rule_instance_is_rejected_for_its_reason() {
    for (name, why) in WHY_RULE {
        let value = load(&rule_dir("invalid").join(name));
        let err = serde_json::from_value::<RuleFile>(value)
            .expect_err(name)
            .to_string();
        assert!(err.contains(why), "{name}: {err:?} should contain {why:?}");
    }
}

#[test]
fn every_invalid_category_instance_is_rejected_for_its_reason() {
    for (name, why) in WHY_CATEGORY {
        let value = load(&category_dir("invalid").join(name));
        let err = serde_json::from_value::<Category>(value)
            .expect_err(name)
            .to_string();
        assert!(err.contains(why), "{name}: {err:?} should contain {why:?}");
    }
}

fn base_rule() -> Value {
    json!({
        "schema_version": 3, "id": "r", "name": "n", "message": "m",
        "match": {"kind": "tokens", "tokens": ["x"]}, "citation": {"source": "s"}
    })
}

#[test]
fn extension_keys_are_ignored_at_every_level() {
    let mut rule = base_rule();
    rule["x-owner"] = json!("docs");
    rule["match"]["x-note"] = json!({"any": ["thing"]});
    rule["citation"]["x-checked"] = json!(true);
    assert!(serde_json::from_value::<RuleFile>(rule).is_ok());
}

#[test]
fn a_judgement_rule_without_severity_is_a_suggestion() {
    let mut rule = base_rule();
    rule["match"] = json!({"kind": "judgement", "question": "Is it so?"});
    let rule = serde_json::from_value::<RuleFile>(rule)
        .unwrap()
        .into_rule();
    assert_eq!(rule.severity, slopvac_model::Severity::Suggestion);
}

#[test]
fn a_rule_without_severity_is_a_warning() {
    let rule = serde_json::from_value::<Rule>(base_rule()).unwrap();
    assert_eq!(rule.severity, slopvac_model::Severity::Warning);
}

#[test]
fn a_null_member_is_rejected_where_the_schema_has_no_null() {
    let mut rule = base_rule();
    rule["fix"] = Value::Null;
    assert!(serde_json::from_value::<Rule>(rule.clone()).is_err());
    assert!(!generated(&rule_schema()).is_valid(&rule));
}

#[test]
fn a_repeated_token_is_rejected_like_the_schema_does() {
    let mut rule = base_rule();
    rule["match"]["tokens"] = json!(["x", "x"]);
    assert!(serde_json::from_value::<Rule>(rule.clone()).is_err());
    assert!(!generated(&rule_schema()).is_valid(&rule));
}

#[test]
fn an_escaped_backslash_before_u_is_not_a_unicode_escape() {
    let mut rule = base_rule();
    rule["match"] = json!({"kind": "pattern", "pattern": "a\\\\u2014"});
    assert!(serde_json::from_value::<Rule>(rule.clone()).is_ok());
    assert!(generated(&rule_schema()).is_valid(&rule));
}

#[test]
fn a_loaded_category_serializes_to_an_instance_that_loads_again_and_validates() {
    let validator = generated(&category_schema());
    let mut n = 0;
    for path in instances(&category_dir("valid")) {
        let loaded = serde_json::from_value::<Category>(load(&path)).unwrap();
        let written = serde_json::to_value(&loaded).unwrap();
        assert!(
            validator.is_valid(&written),
            "{}: {written}",
            file_name(&path)
        );
        let again = serde_json::from_value::<Category>(written).unwrap();
        assert_eq!(again, loaded, "{}", file_name(&path));
        n += 1;
    }
    assert!(n >= 2);
}

#[test]
fn a_loaded_rule_file_serializes_to_an_instance_that_loads_again() {
    for path in instances(&rule_dir("valid")) {
        let name = file_name(&path);
        if is_embedded(&name) {
            continue;
        }
        let loaded = serde_json::from_value::<RuleFile>(load(&path)).unwrap();
        let written = serde_json::to_value(&loaded).unwrap();
        assert!(
            generated(&rule_file_schema()).is_valid(&written),
            "{name}: {written}"
        );
        assert_eq!(
            serde_json::from_value::<RuleFile>(written).unwrap(),
            loaded,
            "{name}"
        );
    }
}
