//! Output invariants of the claims report and the review plan, written as schema conditions.
//!
//! The producers in `claims` and `review` keep these rules by construction. The schemas state
//! them too, so a consumer or a validator sees a combination that the binary never writes as a
//! contract breach. The conditions only constrain the schema: the wire bytes do not change.

use schemars::Schema;
use serde_json::{Map, Value, json};
use slopvac_model::if_then;

/// `{"properties": {key: {"const": value}}, "required": [key]}`.
fn is(key: &str, value: &Value) -> Value {
    json!({"properties": {key: {"const": value}}, "required": [key]})
}

/// Append conditions to the schema's `allOf` list.
fn all_of(schema: &mut Schema, conditions: Vec<Value>) {
    let object: &mut Map<String, Value> = schema.ensure_object();
    let list = object
        .entry("allOf")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(list) = list {
        list.extend(conditions);
    }
}

/// A changed claim carries its original and a similarity above the alignment floor. An added
/// claim has no original, no removed literal and no negation in an original.
pub(crate) fn claim(schema: &mut Schema) {
    all_of(
        schema,
        vec![
            if_then(
                &is("change", &json!("changed")),
                &json!({
                    "required": ["original", "original_line"],
                    "properties": {"similarity": {"minimum": 0.35}},
                }),
            ),
            if_then(
                &is("change", &json!("added")),
                &json!({"properties": {
                    "original": false,
                    "original_line": false,
                    "similarity": {"const": 0},
                    "literals_removed": {"maxItems": 0},
                    "negations_original": {"const": 0},
                    "negation_changed": {"const": false},
                }}),
            ),
        ],
    );
}

/// A dropped sentence that loses a literal, or that the revision covers below 0.6, is a fact
/// candidate. The other direction depends on rounding at the edge, so the schema leaves it open.
pub(crate) fn dropped(schema: &mut Schema) {
    let candidate = json!({"properties": {"fact_candidate": {"const": true}}});
    all_of(
        schema,
        vec![
            if_then(
                &json!({"properties": {"lost_literals": {"minItems": 1}}, "required": ["lost_literals"]}),
                &candidate,
            ),
            if_then(
                &json!({"properties": {"token_coverage": {"exclusiveMaximum": 0.6}}, "required": ["token_coverage"]}),
                &candidate,
            ),
        ],
    );
}

/// Only a changed item carries the sentence before the change.
pub(crate) fn payload_item(schema: &mut Schema) {
    all_of(
        schema,
        vec![
            if_then(
                &is("role", &json!("changed")),
                &json!({"required": ["original"]}),
            ),
            if_then(
                &json!({
                    "properties": {"role": {"enum": ["added", "dropped", "original_document", "revised_document"]}},
                    "required": ["role"],
                }),
                &json!({"properties": {"original": false}}),
            ),
        ],
    );
}

/// The payload kind decides which items it holds.
pub(crate) fn payload(schema: &mut Schema) {
    all_of(
        schema,
        vec![
            if_then(
                &is("kind", &json!("none")),
                &json!({"properties": {"items": {"maxItems": 0}}}),
            ),
            if_then(
                &is("kind", &json!("passages")),
                &json!({"properties": {"items": {"items": {
                    "properties": {"role": {"enum": ["changed", "added", "dropped"]}},
                }}}}),
            ),
            if_then(
                &is("kind", &json!("document")),
                &json!({"properties": {"items": {
                    "minItems": 2,
                    "maxItems": 2,
                    "prefixItems": [
                        {"properties": {"role": {"const": "original_document"}}},
                        {"properties": {"role": {"const": "revised_document"}}},
                    ],
                }}}),
            ),
        ],
    );
}

/// The review type decides the payload kind and whether a provider runs. A `mechanical` review
/// calls no provider, so it names no model, carries no notice and has no report. A plan that
/// compared the two models names both.
pub(crate) fn plan(schema: &mut Schema) {
    all_of(
        schema,
        vec![
            if_then(
                &is("review_type", &json!("mechanical")),
                &json!({"properties": {
                    "payload": {"properties": {"kind": {"const": "none"}}, "required": ["kind"]},
                    "uses_provider": {"const": false},
                    "author_model": false,
                    "reviewer_model": false,
                    "notices": {"maxItems": 0},
                    "report": false,
                }}),
            ),
            if_then(
                &is("review_type", &json!("update")),
                &json!({"properties": {
                    "payload": {"properties": {"kind": {"const": "passages"}}, "required": ["kind"]},
                    "uses_provider": {"const": true},
                }}),
            ),
            if_then(
                &is("review_type", &json!("rewrite")),
                &json!({"properties": {
                    "payload": {"properties": {"kind": {"const": "document"}}, "required": ["kind"]},
                    "uses_provider": {"const": true},
                }}),
            ),
            if_then(
                &is("reviewer_model_checked", &json!(true)),
                &json!({"required": ["author_model", "reviewer_model"]}),
            ),
        ],
    );
}
