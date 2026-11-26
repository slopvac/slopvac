//! `workflow_data` against the vectors shared with the Action's Python escaper.
//!
//! `scripts/workflow_escape.py` mirrors `workflow_data`, because the Action cannot call
//! it. Both sides read `tests/fixtures/workflow-escape-vectors.json`, so a change
//! to either escaper fails the other side's test.

use serde_json::Value;

const VECTORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/workflow-escape-vectors.json"
));

#[test]
fn workflow_data_returns_the_shared_expected_text() {
    let file: Value = serde_json::from_str(VECTORS).expect("the vector file is JSON");
    let vectors = file["workflow_data"]
        .as_array()
        .expect("workflow_data is an array");
    assert!(vectors.len() >= 50, "the vector file lost its cases");
    for vector in vectors {
        let name = vector["name"].as_str().expect("a vector has a name");
        let input = vector["input"].as_str().expect("a vector has an input");
        let expected = vector["expected"]
            .as_str()
            .expect("a vector has an expected");
        assert_eq!(slopvac_sanitize::workflow_data(input), expected, "{name}");
    }
}
