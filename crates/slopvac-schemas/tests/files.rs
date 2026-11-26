//! `write` and `check` against a scratch repository root.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::fs;
use std::path::Path;

use slopvac_plugins::{ProtocolCodes, protocol_codes};
use slopvac_schemas::{ARTIFACTS, check, write};

const CODES_FILE: &str = "docs/reference/plugin-protocol-codes.json";
const CODES_SCHEMA: &str = "schemas/v3/plugin-protocol-codes.schema.json";

fn scratch() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("schemas/v3")).unwrap();
    root
}

#[test]
fn check_passes_on_the_files_write_produced() {
    let root = scratch();
    let written = write(root.path()).unwrap();
    assert_eq!(written.len(), ARTIFACTS.len() + 1);
    assert_eq!(check(root.path()).unwrap(), []);
}

#[test]
fn check_reports_an_edited_file_with_a_diff_of_the_edit() {
    let root = scratch();
    write(root.path()).unwrap();
    let path = root.path().join("schemas/v3/common.schema.json");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("\"minimum\": 1", "\"minimum\": 0")).unwrap();
    let drift = check(root.path()).unwrap();
    assert_eq!(drift.len(), 1);
    assert_eq!(drift[0].file, "common.schema.json");
    assert!(
        drift[0].diff.contains("-          \"minimum\": 0"),
        "{}",
        drift[0].diff
    );
    assert!(
        drift[0].diff.contains("+          \"minimum\": 1"),
        "{}",
        drift[0].diff
    );
}

#[test]
fn write_stores_the_host_code_table_as_the_reference_file() {
    let root = scratch();
    write(root.path()).unwrap();
    let text = fs::read_to_string(root.path().join(CODES_FILE)).unwrap();
    assert!(text.ends_with("}\n"), "{text}");
    let parsed: ProtocolCodes = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, protocol_codes());
}

#[test]
fn the_reference_file_validates_against_its_generated_schema() {
    let root = scratch();
    write(root.path()).unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.path().join(CODES_SCHEMA)).unwrap()).unwrap();
    let file: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.path().join(CODES_FILE)).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(&file));
    let mut extra = file.clone();
    extra["rpc"][0]["surprise"] = true.into();
    assert!(!validator.is_valid(&extra));
}

#[test]
fn check_reports_an_edited_reference_file_with_a_diff_of_the_edit() {
    let root = scratch();
    write(root.path()).unwrap();
    let path = root.path().join(CODES_FILE);
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("\"plugin-usage\"", "\"plugin-use\"")).unwrap();
    let drift = check(root.path()).unwrap();
    assert_eq!(drift.len(), 1);
    assert_eq!(drift[0].file, "plugin-protocol-codes.json");
    assert!(
        drift[0]
            .diff
            .contains("a/docs/reference/plugin-protocol-codes.json"),
        "{}",
        drift[0].diff
    );
    assert!(
        drift[0]
            .diff
            .contains("+      \"slopvac_code\": \"plugin-usage\"")
    );
}

#[test]
fn the_committed_reference_file_and_schema_match_the_host_table() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(check(&repo).unwrap(), []);
}
