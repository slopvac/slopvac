//! Report-level serde behaviour that the generated schema cannot prove.

#![allow(missing_docs)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use slopvac_model::{Gate, Report};

fn report(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v3/conformance/lint-report/valid")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_serialized_report_pins_schema_version_three() {
    let parsed: Report = serde_json::from_value(report("empty-run.json")).unwrap();
    let out = serde_json::to_value(&parsed).unwrap();
    assert_eq!(out["schema_version"], json!(3));
}

#[test]
fn a_gate_rejects_an_empty_reason() {
    let gate = json!({"passed": false, "reasons": [""]});
    assert!(serde_json::from_value::<Gate>(gate).is_err());
}

#[test]
fn a_gate_keeps_a_non_empty_reason() {
    let gate = json!({"passed": false, "reasons": ["too dense"]});
    let parsed: Gate = serde_json::from_value(gate).unwrap();
    assert_eq!(parsed.reasons, vec!["too dense".to_owned()]);
}
