//! The `schemas/v3/conformance/doctor-report/valid` corpus, one test per file. The schema derived
//! from the doctor `Report` type must accept every document the contract calls valid, so a
//! failure means the Rust type cannot describe a valid report.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use schemars::generate::Contract;
use slopvac_cli::doctor::Report;
use slopvac_model::schema_settings;

#[rstest]
fn valid_report_fits_the_derived_schema(
    #[files("../../schemas/v3/conformance/doctor-report/valid/*.json")] path: PathBuf,
) {
    let schema = schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<Report>()
        .to_value();
    let validator = jsonschema::validator_for(&schema).expect("the derived schema compiles");
    let document: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read case")).expect("case parses");
    let errors: Vec<String> = validator
        .iter_errors(&document)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{}: {errors:#?}", path.display());
}
