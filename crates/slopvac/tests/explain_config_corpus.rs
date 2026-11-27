//! The `schemas/v3/conformance/explain-config/valid` corpus, one test per file, and the real
//! output of the binary. The schema derived from the `Document` type must accept every
//! document the contract calls valid and every document the binary prints, so a failure means
//! the Rust type cannot describe one. The derived schema links the shared definitions in
//! `common.schema.json`, which the tests resolve from the published file.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use schemars::generate::Contract;
use serde_json::{Value, json};
use slopvac_cli::explain_config::Document;
use slopvac_model::{schema_id, schema_settings};
use snapbox::cmd::{Command, cargo_bin};

fn validator() -> jsonschema::Validator {
    let mut schema = schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<Document>()
        .to_value();
    schema["$id"] = json!(schema_id("explain-config.schema.json"));
    let common: Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/common.schema.json"),
        )
        .expect("read common"),
    )
    .expect("common parses");
    let registry = jsonschema::Registry::new()
        .add(schema_id("common.schema.json"), common)
        .expect("register common")
        .prepare()
        .expect("registry builds");
    jsonschema::options()
        .with_registry(&registry)
        .build(&schema)
        .expect("the derived schema compiles")
}

fn errors(document: &Value) -> Vec<String> {
    validator()
        .iter_errors(document)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect()
}

#[rstest]
fn valid_document_fits_the_derived_schema(
    #[files("../../schemas/v3/conformance/explain-config/valid/*.json")] path: PathBuf,
) {
    let document: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read case")).expect("case parses");
    let errors = errors(&document);
    assert!(errors.is_empty(), "{}: {errors:#?}", path.display());
}

#[test]
fn the_document_the_binary_prints_fits_the_derived_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("slopvac.toml"),
        "schema_version = 3\nstyles = [\"ste\"]\nlocale = \"en-GB\"\n\n[terms]\nallow = [\"https://host/x\"]\n\n[rules]\n\"core:ste-practices.latin-abbreviation\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n",
    )
    .expect("write config");
    fs::create_dir(dir.path().join("docs")).expect("docs dir");
    fs::write(dir.path().join("docs/a.md"), "# Title\n\nText.\n").expect("write document");
    let assert = Command::new(cargo_bin!("slopvac"))
        .env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir.path())
        .args(["lint", "--explain-config", "--format", "json", "docs/a.md"])
        .assert()
        .code(0);
    let document: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout is one JSON document");
    let errors = errors(&document);
    assert!(errors.is_empty(), "{errors:#?}\n{document}");
}
