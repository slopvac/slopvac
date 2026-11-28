//! Schema-only types for a genre file.
//!
//! The reader (`GenreSpec`) checks a file by hand and keeps its own messages. These types
//! describe the same file so the generator can write `schemas/v3/genre.schema.json`. No code
//! reads a value of them. Each enum list, bound and default comes from the constant or the enum
//! the reader uses, and `slopvac-schemas/tests/genre_agreement.rs` runs the conformance corpus
//! through the reader and through the schema generated here.
//!
//! The name and level definitions and the category dials are the ones in
//! `config.schema.json`. They are taken from `slopvac_config::config_schema` instead of being
//! written again, so the pattern, the level list and the dials have one source. The generator
//! then links each to the config file.

use std::borrow::Cow;
use std::marker::PhantomData;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use slopvac_model::{ContextTag, GenreId, NonEmptyString, SchemaVersion, x_extensions};
use slopvac_rules::schema_generator;

use crate::genre::{Profile, SIGNAL_FIELDS, SIGNAL_RULES, SignalKind};
use crate::schema::{DEFAULT_MIN_SCORE, DEFAULT_WEIGHT, EXCLUSIVE_MINIMUM, MIN_SIGNALS};

/// A definition of `config.schema.json`, as that file's generator wrote it. A name that the
/// config schema does not define gives an empty schema, which the generator then refuses
/// because the definition no longer matches the config file's.
fn config_definition(name: &str) -> Schema {
    slopvac_config::config_schema()
        .get("$defs")
        .and_then(|defs| defs.get(name))
        .cloned()
        .and_then(|definition| Schema::try_from(definition).ok())
        .unwrap_or_else(|| json_schema!({}))
}

/// Declare a unit type whose schema is a definition of `config.schema.json`.
macro_rules! config_definitions {
    ($($(#[$meta:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$meta])*
        struct $name;

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                config_definition(stringify!($name))
            }
        }
    )+};
}

config_definitions! {
    /// A style or genre name, as a bare name or `<plugin>:<name>`.
    Ref,
    /// A rule or category name, as a bare name or `<plugin>:<id>`.
    RuleKey,
    /// The level a rule or category reports at.
    RuleLevel,
}

/// Per-category dials. The definition refers to `RuleLevel`, which a reference to this type
/// cannot register, so register it here.
struct CategorySettings;

impl JsonSchema for CategorySettings {
    fn schema_name() -> Cow<'static, str> {
        "CategorySettings".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        generator.subschema_for::<RuleLevel>();
        config_definition("CategorySettings")
    }
}

/// An object whose keys are rule or category names and whose values are `V`: `propertyNames`
/// takes the `RuleKey` definition and `additionalProperties` takes `V`.
struct Keyed<V>(PhantomData<V>);

impl<V: JsonSchema> JsonSchema for Keyed<V> {
    fn schema_name() -> Cow<'static, str> {
        format!("Keyed{}", V::schema_name()).into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "propertyNames": generator.subschema_for::<RuleKey>(),
            "additionalProperties": generator.subschema_for::<V>(),
        })
    }
}

/// A front matter value that a signal compares with: a string, a number or a boolean.
struct ScalarValue;

impl JsonSchema for ScalarValue {
    fn schema_name() -> Cow<'static, str> {
        "ScalarValue".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": ["string", "number", "boolean"]})
    }
}

/// The closed object of a signal, and the fields that each kind needs and refuses. Each row
/// of [`SIGNAL_RULES`] becomes one `if` on the kinds and a `then` that requires the field the
/// kind needs and refuses the others.
fn signal_object(schema: &mut Schema) {
    x_extensions(schema);
    let conditions: Vec<Schema> = SIGNAL_RULES
        .iter()
        .map(|rule| {
            let kinds: Vec<&str> = rule.kinds.iter().map(|kind| kind.as_str()).collect();
            let kind = match kinds.as_slice() {
                [only] => json_schema!({"const": only}),
                _ => json_schema!({"enum": kinds}),
            };
            let refused: Vec<Schema> = SIGNAL_FIELDS
                .iter()
                .filter(|field| !rule.takes.contains(field))
                .map(|field| json_schema!({"required": [field]}))
                .collect();
            json_schema!({
                "if": {"properties": {"kind": kind}},
                "then": {"required": [rule.needs], "not": {"anyOf": refused}},
            })
        })
        .collect();
    if let Some(all_of) = json_schema!({"allOf": conditions}).get("allOf") {
        schema.insert("allOf".into(), all_of.clone());
    }
}

/// One detection signal. kind selects which other field applies: path takes a glob over the
/// file path, front-matter takes a key and an optional value, heading takes a PCRE2 pattern
/// over heading text, and text takes a PCRE2 pattern over the body.
#[derive(JsonSchema)]
#[schemars(transform = signal_object)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct Signal {
    kind: SignalKind,
    #[schemars(extend("exclusiveMinimum" = EXCLUSIVE_MINIMUM, "default" = DEFAULT_WEIGHT))]
    weight: Option<f64>,
    glob: Option<NonEmptyString>,
    key: Option<NonEmptyString>,
    equals: Option<ScalarValue>,
    pattern: Option<NonEmptyString>,
}

/// Signals that suggest the genre. A document scores the sum of the weights of the signals
/// that match. The genre is a candidate when the score reaches min_score.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
#[expect(
    clippy::doc_markdown,
    reason = "the text is the published description, which names the member without backticks"
)]
struct Detect {
    #[schemars(extend("exclusiveMinimum" = EXCLUSIVE_MINIMUM, "default" = DEFAULT_MIN_SCORE))]
    min_score: Option<f64>,
    #[schemars(extend("minItems" = MIN_SIGNALS))]
    signals: Vec<Signal>,
}

/// A genre file. Unknown keys are errors; keys that start with `x-` are ignored. Semantics are
/// specified in docs/contract/plugins.md.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct GenreDocument {
    schema_version: SchemaVersion,
    id: GenreId,
    title: Option<NonEmptyString>,
    description: Option<NonEmptyString>,
    /// Typical documents, for people.
    surfaces: Option<NonEmptyString>,
    /// Genres this one builds on, lowest first. Their settings apply below this genre's.
    #[schemars(extend("uniqueItems" = true))]
    extends: Option<Vec<Ref>>,
    profile: Option<Profile>,
    detect: Option<Detect>,
    /// Styles the genre activates, in order.
    styles: Option<Vec<Ref>>,
    /// Context tags that every document of this genre carries, whole document, at confidence 1.
    /// A genre that extends this one inherits them. Use it for a context that only a declaration
    /// should set, such as security, which no vocabulary signal may infer.
    #[schemars(extend("uniqueItems" = true))]
    contexts: Option<Vec<ContextTag>>,
    /// Levels keyed by category name. Keys are data, so the x- convention does not apply.
    categories: Option<Keyed<CategorySettings>>,
    /// Levels keyed by rule name. Keys are data, so the x- convention does not apply.
    rules: Option<Keyed<RuleLevel>>,
}

/// Why the genre schema could not be generated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GenreSchemaError {
    /// The config schema has no definition of this name, so the genre schema cannot take it.
    #[error("config.schema.json defines no `{name}`, which the genre schema takes from it")]
    MissingConfigDefinition {
        /// The definition name.
        name: String,
    },
}

/// The definitions of `config.schema.json` that the genre schema takes.
const CONFIG_DEFINITIONS: [&str; 4] = ["Ref", "RuleKey", "RuleLevel", "CategorySettings"];

/// Refuse a config schema that lacks a definition the genre schema takes. Without this check a
/// missing definition would become an unrestricted local one, and the generator links only the
/// definitions that another file owns.
fn check_config_definitions(config: &Schema) -> Result<(), GenreSchemaError> {
    for name in CONFIG_DEFINITIONS {
        if config
            .get("$defs")
            .and_then(|defs| defs.get(name))
            .is_none()
        {
            return Err(GenreSchemaError::MissingConfigDefinition {
                name: name.to_owned(),
            });
        }
    }
    Ok(())
}

/// The schema of a genre file, with its `$defs`. The references to config definitions stay
/// local here, and `slopvac-schemas` points them at the config schema. `slopvac-schemas`
/// writes it to `schemas/v3/genre.schema.json`.
///
/// # Errors
///
/// [`GenreSchemaError::MissingConfigDefinition`] when the config schema lacks a definition
/// that the genre schema takes from it.
pub fn genre_schema() -> Result<Schema, GenreSchemaError> {
    let config =
        Schema::try_from(slopvac_config::config_schema()).unwrap_or_else(|_| json_schema!({}));
    check_config_definitions(&config)?;
    Ok(schema_generator().into_root_schema_for::<GenreDocument>())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{Value, json};

    use super::*;

    /// The kind names that the `Signal` definition states: the `kind` enum, and the kinds
    /// that each per-kind condition (`allOf`) names.
    fn spelled(signal: &Value) -> (BTreeSet<String>, BTreeSet<String>) {
        let names = |v: &Value| -> Vec<String> {
            match v {
                Value::String(one) => vec![one.clone()],
                Value::Array(many) => many
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                _ => Vec::new(),
            }
        };
        let listed = names(&signal["properties"]["kind"]["enum"]);
        let conditions: Vec<String> = signal["allOf"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| {
                let kind = &c["if"]["properties"]["kind"];
                names(if kind.get("const").is_some() {
                    &kind["const"]
                } else {
                    &kind["enum"]
                })
            })
            .collect();
        (
            listed.into_iter().collect(),
            conditions.into_iter().collect(),
        )
    }

    #[test]
    fn the_enum_and_the_conditions_spell_each_kind_as_the_reader_reads_it() {
        let schema = genre_schema().unwrap().to_value();
        let (listed, conditioned) = spelled(&schema["$defs"]["Signal"]);
        assert_eq!(listed.len(), 4, "the schema lists every kind: {listed:?}");
        assert_eq!(
            listed, conditioned,
            "a condition names a kind the enum lacks, or the reverse"
        );
        for name in &listed {
            let read = serde_json::from_value::<SignalKind>(json!(name));
            assert!(
                read.is_ok(),
                "the reader refuses `{name}`, which the schema accepts"
            );
        }
    }

    #[test]
    fn a_config_schema_without_ref_is_refused_by_name() {
        let config =
            json_schema!({"$defs": {"RuleKey": {}, "RuleLevel": {}, "CategorySettings": {}}});
        assert_eq!(
            check_config_definitions(&config),
            Err(GenreSchemaError::MissingConfigDefinition { name: "Ref".into() })
        );
    }

    #[test]
    fn a_config_schema_without_any_definitions_is_refused() {
        assert_eq!(
            check_config_definitions(&json_schema!({})),
            Err(GenreSchemaError::MissingConfigDefinition { name: "Ref".into() })
        );
    }

    #[test]
    fn the_published_config_schema_has_every_definition_the_genre_schema_takes() {
        let config = Schema::try_from(slopvac_config::config_schema()).unwrap();
        assert_eq!(check_config_definitions(&config), Ok(()));
    }
}
