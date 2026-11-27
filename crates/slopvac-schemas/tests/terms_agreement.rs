//! The terms-file and blocklist readers (`slopvac-terms`) and the JSON Schemas for those files
//! give the same verdict on every case of the conformance corpus. The readers keep their own
//! checks and messages; the schemas are what editors and other tools read. Both come from the
//! same constants in `slopvac-terms`, and this test is what proves it, for the generated
//! files and for the committed ones.
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
use slopvac_schemas::generate;
use slopvac_terms::{Blocklist, TermsFile};

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

fn validator(schema: &Value, common: &Value) -> jsonschema::Validator {
    let refs = jsonschema::Registry::new()
        .add(format!("{BASE}common.schema.json"), common)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&refs)
        .build(schema)
        .unwrap()
}

fn cases(kind: &str, outcome: &str) -> Vec<PathBuf> {
    let dir = root()
        .join("schemas/v3/conformance")
        .join(kind)
        .join(outcome);
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.ends_with("cases.toml"))
        .collect();
    assert!(!found.is_empty(), "no {kind}/{outcome} corpus file");
    found.sort();
    found
}

/// The document of a corpus file, whichever of TOML or JSON it is written in.
fn instance(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap();
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => serde_json::from_str(&text).unwrap(),
        _ => toml::from_str(&text).unwrap(),
    }
}

/// What the reader says about a case.
fn reader_accepts(kind: &str, path: &Path) -> bool {
    match kind {
        "terms" => TermsFile::load(path).is_ok(),
        _ => Blocklist::load(path).is_ok(),
    }
}

/// Every case on which the reader and the schema differ, or on which the reader differs from
/// the directory (`valid` or `invalid`) that holds the file.
fn disagreements(kind: &str, schema: &jsonschema::Validator) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = 0;
    for outcome in ["valid", "invalid"] {
        let want = outcome == "valid";
        for path in cases(kind, outcome) {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let id = format!("{kind}/{outcome}/{name}");
            let reader = reader_accepts(kind, &path);
            assert_eq!(reader, want, "{id}: the reader disagrees with the corpus");
            seen += 1;
            let by_schema = schema.is_valid(&instance(&path));
            if by_schema != want {
                found.extend(Some(format!("{id}: reader {reader}, schema {by_schema}")));
            }
        }
    }
    assert!(seen > 0, "no {kind} corpus case ran");
    found
}

#[rstest]
#[case::terms("terms", "terms.schema.json")]
#[case::blocklist("blocklist", "blocklist.schema.json")]
fn the_generated_schema_gives_the_readers_verdict_on_every_case(
    #[case] kind: &str,
    #[case] file: &str,
) {
    let schema = validator(&generated(file), &generated("common.schema.json"));
    assert_eq!(disagreements(kind, &schema), Vec::<String>::new());
}

#[rstest]
#[case::terms("terms", "terms.schema.json")]
#[case::blocklist("blocklist", "blocklist.schema.json")]
fn the_committed_schema_gives_the_readers_verdict_on_every_case(
    #[case] kind: &str,
    #[case] file: &str,
) {
    let schema = validator(&committed(file), &committed("common.schema.json"));
    assert_eq!(disagreements(kind, &schema), Vec::<String>::new());
}
