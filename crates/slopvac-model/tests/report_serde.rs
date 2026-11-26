//! Report-level serde behaviour that the generated schema cannot prove.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use slopvac_model::{ConfigSource, Notice, Report};

const BARE: &str = r#"{"code":"extract-budget","level":"warning","message":"Extraction stopped."}"#;
const WITH_DATA: &str = r#"{"code":"extract-budget","level":"warning","message":"Extraction stopped.","data":{"budget":"segments","limit":1000000}}"#;
const EMPTY_DATA: &str =
    r#"{"code":"extract-budget","level":"warning","message":"Extraction stopped.","data":{}}"#;

fn bare_notice() -> Notice {
    serde_json::from_str(BARE).unwrap()
}

fn budget_data() -> serde_json::Map<String, Value> {
    let Value::Object(map) = json!({"budget": "segments", "limit": 1_000_000}) else {
        unreachable!("the json! literal is an object")
    };
    map
}

#[test]
fn a_notice_with_data_round_trips_and_writes_the_data_object() {
    let parsed: Notice = serde_json::from_str(WITH_DATA).unwrap();
    assert_eq!(parsed.data, budget_data());
    assert_eq!(serde_json::to_string(&parsed).unwrap(), WITH_DATA);
}

#[test]
fn a_notice_without_data_writes_no_data_key() {
    let parsed = bare_notice();
    assert!(parsed.data.is_empty());
    assert_eq!(serde_json::to_string(&parsed).unwrap(), BARE);
}

#[test]
fn a_notice_with_an_empty_data_object_writes_no_data_key() {
    let parsed: Notice = serde_json::from_str(EMPTY_DATA).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), BARE);
}

#[test]
fn with_data_attaches_the_object_to_the_serialized_notice() {
    let notice = bare_notice().with_data(budget_data());
    assert_eq!(serde_json::to_string(&notice).unwrap(), WITH_DATA);
}

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
fn a_config_source_rejects_an_unknown_layer() {
    let source = json!({"layer": "defaults"});
    assert!(serde_json::from_value::<ConfigSource>(source).is_err());
}

#[test]
fn a_config_source_accepts_each_published_layer() {
    for layer in [
        "core",
        "style",
        "genre",
        "extends",
        "project",
        "override",
        "front-matter",
        "directive",
        "cli",
        "env",
    ] {
        let parsed: ConfigSource = serde_json::from_value(json!({"layer": layer})).unwrap();
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            json!({"layer": layer})
        );
    }
}

/// Every `category` value in `value`, in document order.
fn categories(value: &Value) -> Vec<String> {
    match value {
        Value::Object(map) => map
            .iter()
            .flat_map(|(key, item)| match (key.as_str(), item) {
                ("category", Value::String(name)) => vec![name.clone()],
                _ => categories(item),
            })
            .collect(),
        Value::Array(items) => items.iter().flat_map(categories).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn a_report_keeps_a_qualified_category_through_a_round_trip() {
    let before = report("category-qualified.json");
    let parsed: Report = serde_json::from_value(before.clone()).unwrap();
    let after = serde_json::to_value(&parsed).unwrap();
    assert!(categories(&before).iter().any(|c| c == "acme:orwell"));
    assert_eq!(categories(&after), categories(&before));
}
