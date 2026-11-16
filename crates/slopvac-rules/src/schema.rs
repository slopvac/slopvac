//! JSON Schemas generated from the Rust types.
//!
//! They describe what `Deserialize` accepts, so a rule file that validates here also loads.
//! `schemas/v3/rule.schema.json` and `category.schema.json` are the published contract; the
//! parity tests keep the two in step.

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema};
use serde_json::Value;

use crate::model::{Category, Rule, RuleFile};

/// Rule and category members are absent or real values, never `null`: drop the `null`
/// alternative schemars adds for each `Option`.
fn not_nullable(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(types)) = map.get_mut("type") {
                types.retain(|t| t != "null");
                if types.len() == 1 {
                    let only = types.remove(0);
                    map.insert("type".into(), only);
                }
            }
            if let Some(Value::Array(any)) = map.get_mut("anyOf")
                && any
                    .iter()
                    .any(|alt| alt.get("type") == Some(&Value::from("null")))
            {
                any.retain(|alt| alt.get("type") != Some(&Value::from("null")));
                if any.len() == 1 {
                    let only = any.remove(0);
                    map.remove("anyOf");
                    if let Value::Object(inner) = only {
                        map.extend(inner);
                    }
                }
            }
            map.values_mut().for_each(not_nullable);
        }
        Value::Array(items) => items.iter_mut().for_each(not_nullable),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[expect(
    clippy::expect_used,
    reason = "a derived schema always serializes to a schema object"
)]
fn generate<T: JsonSchema>() -> Schema {
    let schema = SchemaSettings::draft2020_12()
        .for_deserialize()
        .into_generator()
        .into_root_schema_for::<T>();
    let mut value = serde_json::to_value(&schema).expect("a schema serializes");
    not_nullable(&mut value);
    Schema::try_from(value).expect("a schema object")
}

/// The schema of a rule as a category file embeds it (`$defs/Rule` in the published schema).
#[must_use]
pub fn rule_schema() -> Schema {
    generate::<Rule>()
}

/// The schema of a standalone rule file, which also requires `schema_version`.
#[must_use]
pub fn rule_file_schema() -> Schema {
    generate::<RuleFile>()
}

/// The schema of a category file.
#[must_use]
pub fn category_schema() -> Schema {
    generate::<Category>()
}
