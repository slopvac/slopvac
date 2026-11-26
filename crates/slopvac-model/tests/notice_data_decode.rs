//! The reader checks the `data` of a typed notice code against the shape that generates the
//! schema, so it refuses what the published schema refuses.
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use rstest::rstest;
use serde_json::{Value, json};
use slopvac_model::Notice;

fn wire(code: &str, data: Option<Value>) -> Value {
    let mut notice = json!({"code": code, "level": "warning", "message": "m"});
    if let Some(data) = data {
        notice["data"] = data;
    }
    notice
}

fn decodes(code: &str, data: Option<Value>) -> bool {
    serde_json::from_value::<Notice>(wire(code, data)).is_ok()
}

#[rstest]
#[case::rules_missing("rule-kind-not-implemented", json!({}))]
#[case::rules_other_key("cases-rule-unchecked", json!({"rule": ["core:a.b"]}))]
#[case::rules_not_an_array("comment-rules-not-run", json!({"rules": "core:a.b"}))]
#[case::rules_item_not_a_string("rule-style-not-implemented", json!({"rules": [1]}))]
#[case::unsupported_kind_missing("extract-unsupported", json!({"document": "a.md"}))]
#[case::unsupported_document_wrong_type("extract-unsupported", json!({"document": 5, "kind": "mermaid"}))]
#[case::budget_limit_missing("extract-budget", json!({"budget": "fragments"}))]
#[case::budget_limit_negative("extract-budget", json!({"budget": "fragments", "limit": -1}))]
#[case::budget_limit_fraction("extract-budget", json!({"budget": "fragments", "limit": 1.5}))]
#[case::budget_name_wrong_type("extract-budget", json!({"budget": 3, "limit": 1}))]
fn a_typed_code_with_data_off_its_shape_is_refused(#[case] code: &str, #[case] data: Value) {
    assert!(!decodes(code, Some(data)));
}

#[rstest]
#[case::rules("comment-rule-named-not-run", json!({"rules": ["core:a.b"], "x-extra": 1, "more": {"a": [1]}}))]
#[case::unsupported("extract-unsupported", json!({"document": "a.md", "kind": "mermaid", "extra": true}))]
#[case::budget("extract-budget", json!({"budget": "segments", "limit": 10, "extra": "k"}))]
fn a_typed_code_with_its_shape_and_extra_keys_is_read(#[case] code: &str, #[case] data: Value) {
    assert!(decodes(code, Some(data)));
}

#[rstest]
#[case::rules("rule-kind-not-implemented")]
#[case::unsupported("extract-unsupported")]
#[case::budget("extract-budget")]
fn a_typed_code_without_data_is_read(#[case] code: &str) {
    assert!(decodes(code, None));
}

#[test]
fn a_code_without_a_shape_keeps_any_object() {
    assert!(decodes("config-unknown-rule", Some(json!({"rule": 5}))));
}
