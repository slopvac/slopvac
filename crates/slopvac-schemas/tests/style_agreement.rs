//! The style readers and the JSON Schema for a style preset give the same verdict on every
//! case of the conformance corpus. Two readers judge a file: `validate_style` in
//! `slopvac-config`, which checks the shape and keeps spans, and `Style::parse` in
//! `slopvac-styles`, which loads the preset. The schema is what editors and other tools read.
//! This test holds all three to one verdict, for the generated file and the committed one.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use serde_json::Value;
use slopvac_config::tree::parse_toml;
use slopvac_config::validate_style;
use slopvac_schemas::generate;
use slopvac_styles::Style;

const BASE: &str = "https://slopvac.github.io/slopvac/schemas/v3/";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn committed(file: &str) -> Value {
    let text = fs::read_to_string(root().join("schemas/v3").join(file)).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn generated(file: &str) -> Value {
    let files = generate().unwrap();
    let found = files.iter().find(|g| g.file == file);
    assert!(found.is_some(), "{file} is not generated");
    serde_json::from_str(&found.unwrap().text).unwrap()
}

/// A validator for `schema` that resolves its references to the other two files.
fn validator(schema: &Value, common: &Value, config: &Value) -> jsonschema::Validator {
    let refs = jsonschema::Registry::new()
        .add(format!("{BASE}common.schema.json"), common)
        .unwrap()
        .add(format!("{BASE}config.schema.json"), config)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&refs)
        .build(schema)
        .unwrap()
}

fn cases(outcome: &str) -> Vec<PathBuf> {
    let dir = root().join("schemas/v3/conformance/style").join(outcome);
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.ends_with("cases.toml"))
        .collect();
    assert!(!found.is_empty(), "no style/{outcome} corpus file");
    found.sort();
    found
}

/// Every case on which a reader and the schema differ, or on which a reader differs from the
/// directory (`valid` or `invalid`) that holds the file.
fn disagreements(schema: &jsonschema::Validator) -> Vec<String> {
    let mut found = Vec::new();
    for outcome in ["valid", "invalid"] {
        let want = outcome == "valid";
        for path in cases(outcome) {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let id = format!("style/{outcome}/{name}");
            let text = fs::read_to_string(&path).unwrap();
            let shape = validate_style(&parse_toml(&text).unwrap(), &text).is_empty();
            let loads = Style::parse(&name, &text).is_ok();
            assert_eq!(
                shape, want,
                "{id}: validate_style disagrees with the corpus"
            );
            assert_eq!(loads, want, "{id}: Style::parse disagrees with the corpus");
            let by_schema = schema.is_valid(&toml::from_str::<Value>(&text).unwrap());
            if by_schema != want {
                found.extend(Some(format!("{id}: readers {want}, schema {by_schema}")));
            }
        }
    }
    found
}

#[rstest]
#[case::generated(generated)]
#[case::committed(committed)]
fn the_schema_gives_the_readers_verdict_on_every_case(#[case] source: fn(&str) -> Value) {
    let schema = validator(
        &source("style.schema.json"),
        &source("common.schema.json"),
        &source("config.schema.json"),
    );
    assert_eq!(disagreements(&schema), Vec::<String>::new());
}
