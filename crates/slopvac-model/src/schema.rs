//! Schema helpers for what `#[derive(JsonSchema)]` attributes cannot say on their own.
//!
//! `slopvac-schemas` writes `schemas/v3` from the Rust types with [`generator`]; the
//! crates that check a document against a generated schema use the same settings. Apply a
//! helper with `#[schemars(transform = ...)]` or `#[schemars(extend("key" = ...))]`.

use schemars::Schema;
use schemars::generate::{Contract, SchemaSettings};
use schemars::transform::RecursiveTransform;
use serde_json::{Map, Value, json};

/// Where the v3 schemas are published; each file's `$id` is this base plus its file name.
pub const BASE: &str = "https://slopvac.github.io/slopvac/schemas/v3/";

/// A lower-case letter, then lower-case letters, digits and hyphens: pack, category, genre
/// and context ids.
pub const SLUG: &str = r"^[a-z][a-z0-9-]*(?![\s\S])";

/// Stable kebab-case: lower-case words of letters and digits joined by single hyphens.
pub const KEBAB: &str = r"^[a-z][a-z0-9]*(-[a-z0-9]+)*(?![\s\S])";

/// A qualified rule id `<pack>:<id>`.
pub const RULE_ID: &str = r"^[a-z][a-z0-9-]*:[A-Za-z0-9][A-Za-z0-9._-]*(?![\s\S])";

/// The `$id` of the published schema file `file`, such as `common.schema.json`.
#[must_use]
pub fn id(file: &str) -> String {
    format!("{BASE}{file}")
}

/// The settings every v3 schema uses: draft 2020-12, stated explicitly so a change of the
/// schemars default cannot move the dialect, plus [`plain_numbers`], [`plain_enums`] and
/// [`one_line_descriptions`] on every subschema.
#[must_use]
pub fn settings(contract: Contract) -> SchemaSettings {
    let mut settings = SchemaSettings::draft2020_12();
    settings.contract = contract;
    settings
        .with_transform(RecursiveTransform(plain_numbers))
        .with_transform(RecursiveTransform(plain_enums))
        .with_transform(RecursiveTransform(one_line_descriptions))
}

/// Join the lines of a doc comment into one line per paragraph, so a description reads the
/// same however the source wraps it.
pub fn one_line_descriptions(schema: &mut Schema) {
    if let Some(Value::String(text)) = schema.get_mut("description") {
        let joined = text
            .split("\n\n")
            .map(|paragraph| paragraph.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n\n");
        *text = joined;
    }
}

/// Closed object with `x-` extension keys: unknown keys are errors, keys that start with
/// `x-` are allowed and ignored.
pub fn x_extensions(schema: &mut Schema) {
    schema.insert("additionalProperties".into(), Value::Bool(false));
    schema.insert("patternProperties".into(), json!({"^x-": true}));
}

/// An optional member is absent or a real value, never `null`: drop the `null` alternative
/// schemars adds for an `Option`, and the `null` default it adds for `#[serde(default)]`.
pub fn absent_not_null(schema: &mut Schema) {
    if schema.get("default") == Some(&Value::Null) {
        schema.remove("default");
    }
    if let Some(Value::Array(types)) = schema.get_mut("type") {
        types.retain(|t| t != "null");
        if types.len() == 1 {
            let only = types.remove(0);
            schema.insert("type".into(), only);
        }
    }
    let null = Value::String("null".into());
    if let Some(Value::Array(any)) = schema.get_mut("anyOf")
        && any.iter().any(|alt| alt.get("type") == Some(&null))
    {
        any.retain(|alt| alt.get("type") != Some(&null));
        if any.len() == 1 {
            let only = any.remove(0);
            schema.remove("anyOf");
            if let Value::Object(map) = only {
                for (k, v) in map {
                    schema.insert(k, v);
                }
            }
        }
    }
}

/// An object with any members: drop the `"additionalProperties": true` schemars writes for a
/// map of JSON values. Without the keyword the schema means the same.
pub fn open_object(schema: &mut Schema) {
    if schema.get("additionalProperties") == Some(&Value::Bool(true)) {
        schema.remove("additionalProperties");
    }
}

/// `{"if": .., "then": ..}`: a conditional for an `allOf` list.
#[must_use]
pub fn if_then(condition: &Value, then: &Value) -> Value {
    json!({"if": condition, "then": then})
}

/// `dependentRequired`: each key names the members that must come with it.
#[must_use]
pub fn dependent_required(pairs: &[(&str, &[&str])]) -> Value {
    Value::Object(
        pairs
            .iter()
            .map(|(key, with)| ((*key).to_owned(), json!(with)))
            .collect::<Map<_, _>>(),
    )
}

/// Drop the `format` schemars adds to Rust number types (`uint`, `double` and the like).
/// They are not JSON Schema formats, and validators only ignore them.
pub fn plain_numbers(schema: &mut Schema) {
    let numeric = match schema.get("type") {
        Some(Value::String(t)) => t == "integer" || t == "number",
        Some(Value::Array(ts)) => ts.iter().any(|t| t == "integer" || t == "number"),
        _ => false,
    };
    if numeric
        && schema
            .get("format")
            .and_then(Value::as_str)
            .is_some_and(is_rust_number_format)
    {
        schema.remove("format");
    }
}

fn is_rust_number_format(format: &str) -> bool {
    matches!(
        format,
        "int" | "uint" | "float" | "double" | "int8" | "uint8" | "int16" | "uint16"
    ) || matches!(
        format,
        "int32" | "uint32" | "int64" | "uint64" | "int128" | "uint128"
    )
}

/// Write a closed set of strings as `{"enum": [...]}`, and a string `const` without `type`.
/// schemars writes a unit enum as a `oneOf` of `const` schemas when its variants carry doc
/// comments, and adds `"type": "string"`; both say the same as the plain form.
pub fn plain_enums(schema: &mut Schema) {
    let consts = schema
        .get("oneOf")
        .and_then(Value::as_array)
        .and_then(|alts| {
            alts.iter()
                .map(|alt| {
                    let obj = alt.as_object()?;
                    let only_const = obj
                        .keys()
                        .all(|k| matches!(k.as_str(), "const" | "type" | "description"));
                    let value = obj.get("const")?;
                    (only_const && value.is_string()).then(|| value.clone())
                })
                .collect::<Option<Vec<_>>>()
        });
    if let Some(values) = consts {
        schema.remove("oneOf");
        schema.insert("enum".into(), Value::Array(values));
    }
    let all_strings = schema
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().all(Value::is_string))
        || schema.get("const").is_some_and(Value::is_string);
    if all_strings && schema.get("type").and_then(Value::as_str) == Some("string") {
        schema.remove("type");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(f: fn(&mut Schema), value: Value) -> Value {
        let mut schema = Schema::try_from(value).unwrap();
        f(&mut schema);
        schema.to_value()
    }

    #[test]
    fn documented_unit_variants_become_a_plain_enum() {
        let got = apply(
            plain_enums,
            json!({"oneOf": [
                {"type": "string", "const": "a", "description": "A."},
                {"type": "string", "const": "b"}
            ]}),
        );
        assert_eq!(got, json!({"enum": ["a", "b"]}));
    }

    #[test]
    fn a_one_of_with_a_real_subschema_is_kept() {
        let input = json!({"oneOf": [{"const": "a"}, {"type": "integer"}]});
        assert_eq!(apply(plain_enums, input.clone()), input);
    }

    #[test]
    fn rust_number_formats_are_dropped_and_real_formats_kept() {
        assert_eq!(
            apply(
                plain_numbers,
                json!({"type": "integer", "format": "uint", "minimum": 0})
            ),
            json!({"type": "integer", "minimum": 0})
        );
        let date = json!({"type": "string", "format": "date-time"});
        assert_eq!(apply(plain_numbers, date.clone()), date);
    }

    #[test]
    fn a_wrapped_doc_comment_becomes_one_line_per_paragraph() {
        assert_eq!(
            apply(
                one_line_descriptions,
                json!({"description": "One\nline.\n\nTwo\n  more."})
            ),
            json!({"description": "One line.\n\nTwo more."})
        );
    }

    #[test]
    fn an_optional_member_loses_only_its_null_alternative() {
        assert_eq!(
            apply(absent_not_null, json!({"type": ["string", "null"]})),
            json!({"type": "string"})
        );
        assert_eq!(
            apply(
                absent_not_null,
                json!({"anyOf": [{"$ref": "#/$defs/Span"}, {"type": "null"}]})
            ),
            json!({"$ref": "#/$defs/Span"})
        );
    }
}
