//! The real output of `slopvac --version --json` validates against the published
//! `schemas/v3/version.schema.json`, with and without an update notice, and a document that
//! breaks the contract does not. A failure here means the binary prints a document that the
//! schema, and so the reader in `slopvac self update`, does not describe.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

const BASE: &str = "https://slopvac.github.io/slopvac/schemas/v3";

fn schemas() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3")
}

/// A validator for the published schema. Its `$ref`s reach `common.schema.json` by URL, so every
/// schema file is registered.
fn validator() -> jsonschema::Validator {
    let mut registry = jsonschema::Registry::new();
    for entry in fs::read_dir(schemas()).expect("read schemas/v3") {
        let path = entry.expect("directory entry").path();
        if path.extension().is_some_and(|e| e == "json") {
            let name = path.file_name().expect("file name").to_string_lossy();
            let schema: Value =
                serde_json::from_str(&fs::read_to_string(&path).expect("read schema"))
                    .expect("schema parses");
            registry = registry
                .add(format!("{BASE}/{name}"), schema)
                .expect("register schema");
        }
    }
    let registry = registry.prepare().expect("registry builds");
    jsonschema::options()
        .with_registry(&registry)
        .build(&json!({ "$ref": format!("{BASE}/version.schema.json") }))
        .expect("the published schema compiles")
}

fn errors(document: &Value) -> Vec<String> {
    validator()
        .iter_errors(document)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect()
}

/// `slopvac` with the variables that switch the update notice off removed.
fn slopvac(cache: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_slopvac"));
    for var in [
        "CI",
        "NO_UPDATE_NOTIFIER",
        "SLOPVAC_NO_UPDATE_CHECK",
        "DO_NOT_TRACK",
        "SLOPVAC_OFFLINE",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("SLOPVAC_CACHE_DIR", cache);
    cmd
}

fn run(cmd: &mut Command) -> Value {
    let out = cmd.output().expect("run slopvac");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    serde_json::from_slice(&out.stdout).expect("stdout is one JSON document")
}

#[test]
fn the_document_the_binary_prints_offline_validates() {
    let cache = tempfile::tempdir().expect("tempdir");
    let document = run(slopvac(cache.path()).args(["--offline", "--version", "--json"]));
    let errors = errors(&document);
    assert!(errors.is_empty(), "{errors:#?}\n{document}");
}

#[test]
fn the_document_the_binary_prints_with_an_update_notice_validates() {
    let cache = tempfile::tempdir().expect("tempdir");
    fs::write(
        cache.path().join("update-check.json"),
        r#"{"checked_at":4102444800,"latest":"99.0.0"}"#,
    )
    .expect("write cache");
    let document = run(slopvac(cache.path()).args(["--version", "--json"]));
    assert_eq!(
        document["notices"][0]["x-latest"], "99.0.0",
        "the cached release shows as a notice: {document}"
    );
    let errors = errors(&document);
    assert!(errors.is_empty(), "{errors:#?}\n{document}");
}

#[test]
fn a_document_without_schema_version_does_not_validate() {
    let cache = tempfile::tempdir().expect("tempdir");
    let mut document = run(slopvac(cache.path()).args(["--offline", "--version", "--json"]));
    document
        .as_object_mut()
        .expect("the document is an object")
        .remove("schema_version");
    assert_eq!(
        errors(&document),
        [r#": "schema_version" is a required property"#]
    );
}
