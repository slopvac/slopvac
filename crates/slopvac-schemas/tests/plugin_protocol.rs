//! The generated plugin protocol schema defines the result of every method.

use std::error::Error;
use std::fs;
use std::path::Path;

use serde_json::Value;

/// The definitions that hold the results of the methods, as the published schema names them.
const RESULTS: [&str; 9] = [
    "InitializeResult",
    "TagResult",
    "ExtractResult",
    "RenderResult",
    "FixResult",
    "ScoreResult",
    "CompleteResult",
    "RecipeResult",
    "BinaryExtractResult",
];

#[test]
fn every_method_result_has_a_definition_in_the_published_schema() -> Result<(), Box<dyn Error>> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/plugin-protocol.schema.json");
    let schema: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    let defs = schema["$defs"]
        .as_object()
        .ok_or("the schema has no $defs")?;
    let missing: Vec<_> = RESULTS
        .iter()
        .filter(|name| !defs.contains_key(**name))
        .collect();
    assert!(
        missing.is_empty(),
        "add them to `keep` in artifacts.rs: {missing:?}"
    );
    Ok(())
}
