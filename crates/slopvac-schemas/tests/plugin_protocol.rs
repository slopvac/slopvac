//! The generated plugin protocol schema keeps the definition of every method result.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::fs;
use std::path::Path;

use serde_json::Value;

#[test]
fn every_method_result_has_a_definition_in_the_published_schema() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/plugin-protocol.schema.json");
    let schema: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let defs = schema["$defs"].as_object().unwrap();
    let results = slopvac_plugins::result_defs();
    assert!(results.iter().any(|name| name == "InitializeResult"));
    let missing: Vec<_> = results
        .iter()
        .filter(|name| !defs.contains_key(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "add them to `keep` in artifacts.rs: {missing:?}"
    );
}
