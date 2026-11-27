//! The document that `slopvac lint --explain-config --format json` prints: the resolved
//! settings of each input, with the layer that set each one.
//!
//! The types are the producer contract of `schemas/v3/explain-config.schema.json`, which
//! `slopvac-schemas` generates from them. The text form prints the same entries. Semantics are
//! in `docs/contract/config.md`, section 1.4.1.

use std::borrow::Cow;
use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Serialize;
use slopvac_config::Explained;
use slopvac_engine::{Level, Settings};
use slopvac_model::{
    ConfigSource, GenreSource, NonEmptyString, RuleId, SchemaVersion, Severity, x_extensions,
};

use crate::lint_settings::RuleOrigin;
use crate::rule_catalog::Catalog;

/// The document: one record per input, in argument order. Field order is the JSON key order.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct Document {
    /// Always 3.
    schema_version: SchemaVersion,
    /// Version of the tool that wrote the document. Not a schema version.
    slopvac_version: NonEmptyString,
    /// The inputs that resolved, in argument order. An input that could not be read is left
    /// out, and the run exits 2.
    inputs: Vec<Input>,
}

impl Document {
    /// A document for `inputs`.
    #[must_use]
    pub fn new(inputs: Vec<Input>) -> Self {
        Self {
            schema_version: SchemaVersion,
            slopvac_version: NonEmptyString::new(env!("CARGO_PKG_VERSION"))
                .expect("the package version is not empty"),
            inputs,
        }
    }

    /// The document as pretty-printed JSON with a final newline.
    ///
    /// # Errors
    /// When serialization fails, which the types here cannot cause.
    pub fn render(&self) -> Result<String, serde_json::Error> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }
}

/// The resolved config of one input.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct Input {
    /// The input as a report names it: relative to the working directory, `<external>/<file
    /// name>` outside it, `<stdin>` for standard input.
    path: ShownPath,
    /// The directory of the nearest config: `.`, a path relative to the working directory, or
    /// `<external>/<name>`.
    config_root: ShownPath,
    /// The built-in defaults, then each config file, lowest layer first, as the lint report
    /// lists them.
    config_sources: Vec<ConfigSource>,
    /// The profile name.
    #[schemars(length(min = 1))]
    profile: String,
    /// The genre the config sets. The genre that detection picks reads the document, so it is
    /// not resolved here.
    genre: Genre,
    /// Each resolved setting, in key order.
    settings: Vec<Setting>,
    /// How many rules there are, and how many a layer gave a level.
    rules: RuleCounts,
    /// The rules whose level a layer set, sorted by id.
    configured_rules: Vec<ConfiguredRule>,
    /// Whether front matter took part in resolution. False until it does. When it does, the
    /// document gains key names and their source locations, never values.
    front_matter_resolved: bool,
}

impl Input {
    /// The record of the input shown as `path`.
    ///
    /// `explained` holds the settings with their values already redacted, `settings` and
    /// `origins` come from the same build that lint uses, and `config_root` is the root as the
    /// text form shows it.
    #[must_use]
    pub(crate) fn new(
        path: &str,
        config_root: &str,
        settings: &Settings,
        explained: &[Explained],
        origins: &[RuleOrigin],
        catalog: &Catalog,
    ) -> Self {
        let mut counts = LevelCounts::default();
        let mut configured: Vec<ConfiguredRule> = origins
            .iter()
            .map(|origin| {
                let level = ConfiguredLevel::of(origin.level);
                counts.add(level);
                ConfiguredRule {
                    id: origin.id.clone(),
                    level,
                    source: origin.source.clone(),
                }
            })
            .collect();
        configured.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        Self {
            path: ShownPath(path.to_owned()),
            config_root: ShownPath(config_root.to_owned()),
            config_sources: settings.config_sources.clone(),
            profile: settings.profile.as_str().to_owned(),
            genre: Genre {
                id: settings.genre.id.as_str().to_owned(),
                source: settings.genre.source,
            },
            settings: explained.iter().map(Setting::of).collect(),
            rules: RuleCounts {
                total: catalog.rules().count(),
                configured: counts,
            },
            configured_rules: configured,
            front_matter_resolved: false,
        }
    }
}

/// A path made safe to show: never absolute.
#[derive(Debug, Serialize)]
#[serde(transparent)]
struct ShownPath(String);

impl JsonSchema for ShownPath {
    fn schema_name() -> Cow<'static, str> {
        "ShownPath".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A path made safe to show: relative to the working directory (`.` for the directory itself), `<external>/<file name>` outside it, or `<stdin>`. Never absolute (policies.md, section 1.7).",
            "type": "string",
            "minLength": 1,
            "pattern": "^(?!/)"
        })
    }
}

/// The genre the config sets.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Genre {
    /// The genre id.
    #[schemars(length(min = 1))]
    id: String,
    /// `explicit` when a config layer sets the genre, `default` otherwise.
    source: GenreSource,
}

/// One resolved setting.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Setting {
    /// The key path, such as `thresholds.min_score`.
    #[schemars(length(min = 1))]
    key: String,
    /// The final value, as configured. A URL has no user-info, query or fragment.
    value: ConfigValue,
    /// The layer and location that set the final value.
    source: ConfigSource,
    /// The values the final value replaced, highest layer first.
    replaced: Vec<Replaced>,
}

impl Setting {
    fn of(entry: &Explained) -> Self {
        Self {
            key: entry.path.clone(),
            value: ConfigValue::of(&entry.value),
            source: entry.source.clone(),
            replaced: entry
                .overridden
                .iter()
                .rev()
                .map(|c| Replaced {
                    value: ConfigValue::of(&c.value),
                    source: c.source.clone(),
                })
                .collect(),
        }
    }
}

/// A value a setting replaced.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Replaced {
    /// The replaced value, as configured.
    value: ConfigValue,
    /// The layer and location that set it.
    source: ConfigSource,
}

/// A setting value as configured: any JSON value.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
enum ConfigValue {
    /// No value.
    Null(()),
    /// A flag.
    Bool(bool),
    /// A number.
    Number(serde_json::Number),
    /// Text.
    Text(String),
    /// A list.
    List(Vec<ConfigValue>),
    /// A table.
    Table(BTreeMap<String, ConfigValue>),
}

impl ConfigValue {
    fn of(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null(()),
            serde_json::Value::Bool(b) => Self::Bool(*b),
            serde_json::Value::Number(n) => Self::Number(n.clone()),
            serde_json::Value::String(s) => Self::Text(s.clone()),
            serde_json::Value::Array(items) => Self::List(items.iter().map(Self::of).collect()),
            serde_json::Value::Object(map) => {
                Self::Table(map.iter().map(|(k, v)| (k.clone(), Self::of(v))).collect())
            }
        }
    }
}

/// How many rules there are, and how many of them a layer gave a level.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct RuleCounts {
    /// Every rule in the catalogue. A rule no layer named keeps the level of its definition
    /// and its genre.
    total: usize,
    /// The rules a layer gave a level, by that level.
    configured: LevelCounts,
}

/// Rules by configured level.
#[derive(Debug, Default, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct LevelCounts {
    /// Rules set to `error`.
    error: usize,
    /// Rules set to `warning`.
    warning: usize,
    /// Rules set to `suggestion`.
    suggestion: usize,
    /// Rules set to `off`.
    off: usize,
}

impl LevelCounts {
    fn add(&mut self, level: ConfiguredLevel) {
        match level {
            ConfiguredLevel::Error => self.error += 1,
            ConfiguredLevel::Warning => self.warning += 1,
            ConfiguredLevel::Suggestion => self.suggestion += 1,
            ConfiguredLevel::Off => self.off += 1,
        }
    }
}

/// A rule whose level a layer set.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct ConfiguredRule {
    /// The qualified id: `core:<category>.<rule>` or `<plugin>:<category>.<rule>`.
    id: RuleId,
    /// The level lint applies.
    level: ConfiguredLevel,
    /// The layer and location that set the level.
    source: ConfigSource,
}

/// A level a layer can give a rule. The set grows, so a consumer tolerates a new value.
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline, extend("x-slopvac-extensible" = true))]
enum ConfiguredLevel {
    /// Findings fail the run.
    Error,
    /// Findings count and are reported.
    Warning,
    /// Findings are advice.
    Suggestion,
    /// The rule does not run.
    Off,
}

impl ConfiguredLevel {
    fn of(level: Level) -> Self {
        match level {
            Level::Off => Self::Off,
            Level::On(Severity::Error) => Self::Error,
            Level::On(Severity::Warning) => Self::Warning,
            Level::On(Severity::Suggestion) => Self::Suggestion,
        }
    }
}
