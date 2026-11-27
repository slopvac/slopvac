//! The reader and the JSON Schema give the same verdict on every case of the conformance
//! corpus. The reader (`Shape`) validates a document and keeps its spans; the schema, written
//! from the types in `wire.rs`, is what editors and other tools read. Both come from the same
//! constants, and this test is what proves it. A check that JSON Schema cannot state, such as a
//! name that must appear once, goes on `SHAPE_ONLY` with the reason.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use slopvac_config::tree::{parse_toml, parse_yaml};
use slopvac_config::{config_schema, validate_config, validate_frontmatter};

const BASE: &str = "https://slopvac.github.io/slopvac/schemas/v3/";

/// Corpus files that only the reader refuses, each with what JSON Schema cannot say.
const SHAPE_ONLY: &[(&str, &str)] = &[];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn published(file: &str) -> Value {
    let text = fs::read_to_string(root().join("schemas/v3").join(file)).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn validator(schema: &Value) -> jsonschema::Validator {
    let mut refs = jsonschema::Registry::new();
    for file in ["common.schema.json", "config.schema.json"] {
        refs = refs.add(format!("{BASE}{file}"), published(file)).unwrap();
    }
    jsonschema::options()
        .with_registry(&refs.prepare().unwrap())
        .build(schema)
        .unwrap()
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "toml" || x == "yaml") && !p.ends_with("cases.toml")
        })
        .collect();
    out.sort();
    out
}

/// What the reader says about a case, and what the schema says.
fn verdicts(kind: &str, path: &Path, schema: &jsonschema::Validator) -> (bool, bool) {
    let text = fs::read_to_string(path).unwrap();
    let (reader, instance) = if kind == "config" {
        let tree = parse_toml(&text).unwrap();
        (validate_config(&tree, &text).is_empty(), tree.to_json())
    } else {
        let tree = parse_yaml(&text).unwrap();
        (
            validate_frontmatter(&tree, &text).is_empty(),
            tree.to_json(),
        )
    };
    (reader, schema.is_valid(&instance))
}

fn disagreements(kind: &str, schema: &jsonschema::Validator) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = 0;
    for dir in ["valid", "invalid"] {
        let want = dir == "valid";
        let cases = root().join("schemas/v3/conformance").join(kind).join(dir);
        for path in files(&cases) {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let (reader, by_schema) = verdicts(kind, &path, schema);
            seen += 1;
            assert_eq!(
                reader, want,
                "{kind}/{dir}/{name}: the reader disagrees with the corpus"
            );
            let id = format!("{kind}/{dir}/{name}");
            let shape_only = SHAPE_ONLY.iter().any(|(file, _)| *file == id);
            let expected = if shape_only { !want } else { want };
            if by_schema != expected {
                out.push(format!("{id}: reader {reader}, schema {by_schema}"));
            }
        }
    }
    assert!(seen > 0, "no {kind} corpus case ran");
    out
}

#[test]
fn the_published_config_schema_gives_the_readers_verdict_on_every_case() {
    let schema = validator(&published("config.schema.json"));
    assert_eq!(disagreements("config", &schema), Vec::<String>::new());
}

#[test]
fn the_generated_config_schema_gives_the_readers_verdict_on_every_case() {
    let schema = validator(&config_schema());
    assert_eq!(disagreements("config", &schema), Vec::<String>::new());
}

#[test]
fn the_published_front_matter_schema_gives_the_readers_verdict_on_every_case() {
    let schema = validator(&published("frontmatter.schema.json"));
    assert_eq!(disagreements("frontmatter", &schema), Vec::<String>::new());
}
