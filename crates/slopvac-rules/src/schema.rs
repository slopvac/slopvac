//! JSON Schemas generated from the Rust types.
//!
//! They describe what `Deserialize` accepts, so a rule file that validates here also loads.
//! `schemas/v3/rule.schema.json` and `category.schema.json` are written from the same types
//! by the `slopvac-schemas` generator with [`schema_generator`].

use schemars::generate::Contract;
use schemars::transform::{RecursiveTransform, Transform};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde_json::{Value, json};
use slopvac_model::schema::{absent_not_null, settings};

use crate::model::{Category, Rule, RuleFile};

/// The generator for rule and category schemas: the shared v3 settings, for what
/// `Deserialize` accepts. Rule and category members are absent or real values, never `null`,
/// so every `Option` loses the `null` alternative schemars adds.
#[must_use]
pub fn schema_generator() -> SchemaGenerator {
    settings(Contract::Deserialize)
        .with_transform(RecursiveTransform(absent_not_null))
        .into_generator()
}

fn generate<T: JsonSchema>() -> Schema {
    schema_generator().into_root_schema_for::<T>()
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

/// Write a map whose keys must match a pattern as `propertyNames` plus `additionalProperties`,
/// the published form, instead of the single `patternProperties` entry plus
/// `"additionalProperties": false` that schemars writes. Both accept the same objects.
pub(crate) fn property_names(schema: &mut Schema) {
    let single = schema
        .get("patternProperties")
        .and_then(Value::as_object)
        .filter(|patterns| patterns.len() == 1)
        .and_then(|patterns| patterns.iter().next())
        .map(|(pattern, values)| (pattern.clone(), values.clone()));
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    if let Some((pattern, values)) = single
        && closed
        && schema.get("properties").is_none()
    {
        schema.remove("patternProperties");
        schema.insert("propertyNames".into(), json!({ "pattern": pattern }));
        schema.insert("additionalProperties".into(), values);
    }
}

/// Write the alternatives of an untagged enum as `oneOf`, the published form. schemars writes
/// `anyOf`; the two agree when no value matches two alternatives.
pub(crate) fn exclusive_alternatives(schema: &mut Schema) {
    if let Some(alternatives) = schema.remove("anyOf") {
        schema.insert("oneOf".into(), alternatives);
    }
}

/// The `kind` tag of a `Match` payload: a required member that holds exactly this name. The
/// payload structs do not carry the tag themselves; `Match` reads it.
pub(crate) struct Kind(pub(crate) &'static str);

impl Transform for Kind {
    fn transform(&mut self, schema: &mut Schema) {
        if let Some(Value::Object(properties)) = schema.get_mut("properties") {
            properties.insert("kind".into(), json!({ "const": self.0 }));
        }
        match schema.get_mut("required") {
            Some(Value::Array(required)) => required.insert(0, "kind".into()),
            _ => {
                schema.insert("required".into(), json!(["kind"]));
            }
        }
    }
}
