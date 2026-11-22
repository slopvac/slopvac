//! `slopvac rules --format json` and `slopvac explain --format json` against the published
//! `rules-output` schema (`schemas/v3/rules-output.schema.json`).
//!
//! Each test runs the built binary and validates what it prints. The negative checks show
//! that the schema rejects documents of the authoring shape, so a pass is not vacuous.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::path::PathBuf;

use assert_cmd::Command;
use jsonschema::Validator;
use rstest::rstest;
use serde_json::{Value, json};

const SCHEMA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/v3");
const OUTPUT_ID: &str = "https://slopvac.github.io/slopvac/schemas/v3/rules-output.schema.json";
const CHAT_LEAKAGE: &str = "core:ai-residue.chat-leakage";

fn read_schema(name: &str) -> Value {
    let path = PathBuf::from(SCHEMA_DIR).join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("schema file")).expect("schema json")
}

/// A validator for `schema`, with every schema file of the directory that it may reference.
fn validator(schema: &Value) -> Validator {
    let mut registry = jsonschema::Registry::new();
    for name in ["common", "lint-report", "rule", "rules-output"] {
        let doc = read_schema(&format!("{name}.schema.json"));
        let id = doc["$id"].as_str().expect("$id").to_owned();
        registry = registry.add(id, doc).expect("register schema");
    }
    let registry = registry.prepare().expect("prepare registry");
    jsonschema::options()
        .with_registry(&registry)
        .build(schema)
        .expect("compile schema")
}

fn listing_validator() -> Validator {
    validator(&json!({ "$ref": OUTPUT_ID }))
}

fn explain_validator() -> Validator {
    validator(&json!({ "$ref": format!("{OUTPUT_ID}#/$defs/ExplainOutput") }))
}

fn rejects(validator: &Validator, instance: &Value) -> bool {
    !validator.is_valid(instance)
}

fn errors(validator: &Validator, instance: &Value) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect()
}

fn run(args: &[&str]) -> Value {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    cmd.env("SLOPVAC_NO_UPDATE_CHECK", "1").env("NO_COLOR", "1");
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd.env_remove(var);
    }
    // From `/` no config is found, so a run without `--config` sees the rules as shipped.
    cmd.current_dir("/");
    let out = cmd.args(args).assert().code(0).get_output().stdout.clone();
    serde_json::from_slice(&out).expect("json output")
}

/// A config file in a fresh directory. The directory lives as long as the guard.
fn config(text: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("slopvac.toml");
    std::fs::write(&file, text).expect("write config");
    let path = file.to_str().expect("utf-8 path").to_owned();
    (dir, path)
}

const CHAT_LEAKAGE_OFF: &str =
    "schema_version = 3\n\n[rules]\n\"ai-residue.chat-leakage\" = \"off\"\n";

#[test]
fn the_rules_listing_validates() {
    let listing = run(&["rules", "--format", "json"]);
    assert_eq!(errors(&listing_validator(), &listing), Vec::<String>::new());
    assert_eq!(listing["schema_version"], 3);
}

#[rstest]
#[case::pattern_with_an_exception_and_a_suppression(CHAT_LEAKAGE)]
#[case::judgement("core:ai-tells-content-shape.fabricated-citations-remainder")]
#[case::terms_plugin("terms:terms.casing")]
fn explain_validates(#[case] id: &str) {
    let rule = run(&["explain", id, "--format", "json"]);
    assert_eq!(errors(&explain_validator(), &rule), Vec::<String>::new());
    assert_eq!(rule["schema_version"], 3);
    assert_eq!(rule["id"], id);
}

#[test]
fn a_listing_filtered_to_nothing_still_validates() {
    let listing = run(&["rules", "--genre", "no-such-genre", "--format", "json"]);
    assert_eq!(listing["rules"], json!([]));
    assert_eq!(errors(&listing_validator(), &listing), Vec::<String>::new());
}

#[test]
fn a_configured_severity_validates_in_the_listing_and_in_explain() {
    let (_dir, path) = config(CHAT_LEAKAGE_OFF);

    let listing = run(&[
        "rules",
        "--category",
        "ai-residue",
        "--format",
        "json",
        "--config",
        &path,
    ]);
    assert_eq!(errors(&listing_validator(), &listing), Vec::<String>::new());
    let changed = listing["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == CHAT_LEAKAGE)
        .expect("the rule is listed");
    assert_eq!(changed["severity"], "error");
    assert_eq!(changed["effective_severity"], "off");
    assert!(changed["severity_from"].is_string(), "{changed}");

    let explained = run(&[
        "explain",
        CHAT_LEAKAGE,
        "--format",
        "json",
        "--config",
        &path,
    ]);
    assert_eq!(
        errors(&explain_validator(), &explained),
        Vec::<String>::new()
    );
    assert_eq!(explained["effective_severity"], "off");
    assert!(explained["severity_from"].is_string(), "{explained}");
}

#[test]
fn a_category_file_is_not_a_rules_listing() {
    let category = read_category_fixture();
    assert!(rejects(&listing_validator(), &category));
}

#[test]
fn an_unqualified_rule_id_is_rejected() {
    let mut listing = run(&["rules", "--category", "ai-residue", "--format", "json"]);
    listing["rules"][0]["id"] = json!("chat-leakage");
    let found = errors(&listing_validator(), &listing);
    assert!(
        found.iter().any(|e| e.ends_with("at /rules/0/id")),
        "{found:?}"
    );
}

#[test]
fn explain_output_without_schema_version_is_rejected() {
    let mut rule = run(&["explain", CHAT_LEAKAGE, "--format", "json"]);
    rule.as_object_mut().unwrap().remove("schema_version");
    assert!(rejects(&explain_validator(), &rule));
}

#[test]
fn a_listing_entry_with_an_unknown_key_is_rejected() {
    let mut listing = run(&["rules", "--category", "ai-residue", "--format", "json"]);
    listing["rules"][0]["colour"] = json!("red");
    let found = errors(&listing_validator(), &listing);
    assert!(
        found.iter().any(|e| e.ends_with("at /rules/0")),
        "{found:?}"
    );
}

/// The authoring file of one category, from the converted core rules.
fn read_category_fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../slopvac-rules/tests/fixtures/core-v3/ai-residue.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("fixture json")
}
