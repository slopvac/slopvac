//! The `schemas/v3/conformance/doctor-report/valid` corpus, one test per file. The schema derived
//! from the doctor `Report` type must accept every document the contract calls valid, so a
//! failure means the Rust type cannot describe a valid report. The derived schema links the
//! shared definitions in `common.schema.json`, which the test resolves from the published file.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use schemars::generate::Contract;
use serde_json::{Value, json};
use slopvac_cli::doctor::Report;
use slopvac_model::{schema_id, schema_settings};

#[rstest]
fn valid_report_fits_the_derived_schema(
    #[files("../../schemas/v3/conformance/doctor-report/valid/*.json")] path: PathBuf,
) {
    let mut schema = schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<Report>()
        .to_value();
    schema["$id"] = json!(schema_id("doctor-report.schema.json"));
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
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&schema)
        .expect("the derived schema compiles");
    let document: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read case")).expect("case parses");
    let errors: Vec<String> = validator
        .iter_errors(&document)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{}: {errors:#?}", path.display());
}
