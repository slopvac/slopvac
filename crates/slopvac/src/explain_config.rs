//! The document that `slopvac lint --explain-config --format json` prints: the resolved
//! settings of each input, with the layer that set each one.
//!
//! The types are the producer contract of `schemas/v3/explain-config.schema.json`, which
//! `slopvac-schemas` generates from them. The text form prints the same entries. Semantics are
//! in `docs/contract/config.md`, section 1.4.1.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Serialize;
use slopvac_config::Explained;
use slopvac_engine::{Level, Settings};
use slopvac_model::{
    ConfigSource, GenreSource, NonEmptyString, RuleId, SchemaVersion, Severity, x_extensions,
};

use crate::lint_settings::{RuleOrigin, redacted};
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
    inputs: Vec<ExplainInput>,
}

impl Document {
    /// A document for `inputs`.
    #[must_use]
    pub fn new(inputs: Vec<ExplainInput>) -> Self {
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
pub struct ExplainInput {
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
    /// The genre whose settings the record holds, as the lint report names it for the same
    /// input.
    genre: ExplainGenre,
    /// Each resolved setting, in key order.
    settings: Vec<ExplainSetting>,
    /// How many rules there are, and how many a layer gave a level.
    rules: ExplainRuleCounts,
    /// The rules whose level a layer set, sorted by id.
    configured_rules: Vec<ExplainConfiguredRule>,
    /// Whether the input's front matter and inline directives took part in resolution. False
    /// when `[frontmatter] enabled = false` turns both off.
    front_matter_resolved: bool,
}

impl ExplainInput {
    /// The record of the input shown as `path`.
    ///
    /// `explained` holds the settings with their values already redacted, `settings` and
    /// `origins` come from the same build that lint uses, and `config_root` is the root as the
    /// text form shows it. `root` is the config root itself: the file of every origin shows
    /// under it, or as `<external>/<file name>` outside it (policies.md, section 1.7).
    #[must_use]
    pub(crate) fn new(
        path: &str,
        config_root: &str,
        root: &Path,
        settings: &Settings,
        explained: &[Explained],
        origins: &[RuleOrigin],
        catalog: &Catalog,
    ) -> Self {
        let mut counts = ExplainLevelCounts::default();
        let mut configured: Vec<ExplainConfiguredRule> = origins
            .iter()
            .map(|origin| {
                let level = ConfiguredLevel::of(origin.level);
                counts.add(level);
                ExplainConfiguredRule {
                    id: origin.id.clone(),
                    level,
                    source: redacted(&origin.source, root),
                }
            })
            .collect();
        configured.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        Self {
            path: ShownPath(path.to_owned()),
            config_root: ShownPath(config_root.to_owned()),
            config_sources: settings
                .config_sources
                .iter()
                .map(|source| redacted(source, root))
                .collect(),
            profile: settings.profile.as_str().to_owned(),
            genre: ExplainGenre {
                id: settings.genre.id.as_str().to_owned(),
                source: settings.genre.source,
            },
            settings: explained
                .iter()
                .map(|entry| ExplainSetting::of(entry, root))
                .collect(),
            rules: ExplainRuleCounts {
                total: catalog.rules().count(),
                configured: counts,
            },
            configured_rules: configured,
            front_matter_resolved: false,
        }
    }

    /// The same record, saying whether the input's front matter and inline directives took
    /// part in resolution.
    #[must_use]
    pub(crate) fn with_front_matter_resolved(mut self, resolved: bool) -> Self {
        self.front_matter_resolved = resolved;
        self
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
struct ExplainGenre {
    /// The genre id.
    #[schemars(length(min = 1))]
    id: String,
    /// `explicit` when a layer names the genre, `detected` when genre detection chose it,
    /// `default` otherwise.
    source: GenreSource,
}

/// One resolved setting.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct ExplainSetting {
    /// The key path, such as `thresholds.min_score`.
    #[schemars(length(min = 1))]
    key: String,
    /// The final value, as configured. A URL has no user-info, query or fragment.
    value: ExplainValue,
    /// The layer and location that set the final value.
    source: ConfigSource,
    /// The values the final value replaced, highest layer first.
    replaced: Vec<ExplainReplaced>,
}

impl ExplainSetting {
    fn of(entry: &Explained, root: &Path) -> Self {
        Self {
            key: entry.path.clone(),
            value: ExplainValue::of(&entry.value),
            source: redacted(&entry.source, root),
            replaced: entry
                .overridden
                .iter()
                .rev()
                .map(|c| ExplainReplaced {
                    value: ExplainValue::of(&c.value),
                    source: redacted(&c.source, root),
                })
                .collect(),
        }
    }
}

/// A value a setting replaced.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct ExplainReplaced {
    /// The replaced value, as configured.
    value: ExplainValue,
    /// The layer and location that set it.
    source: ConfigSource,
}

/// A setting value as configured: null, a flag, a number, text, a list or a table.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
enum ExplainValue {
    /// No value.
    Null(()),
    /// A flag.
    Bool(bool),
    /// A number.
    Number(serde_json::Number),
    /// Text.
    Text(String),
    /// A list.
    List(Vec<ExplainValue>),
    /// A table.
    Table(BTreeMap<String, ExplainValue>),
}

impl ExplainValue {
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
struct ExplainRuleCounts {
    /// Every rule in the catalogue. A rule no layer named keeps the level of its definition
    /// and its genre.
    total: usize,
    /// The rules a layer gave a level, by that level.
    configured: ExplainLevelCounts,
}

/// Rules by configured level.
#[derive(Debug, Default, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct ExplainLevelCounts {
    /// Rules set to `error`.
    error: usize,
    /// Rules set to `warning`.
    warning: usize,
    /// Rules set to `suggestion`.
    suggestion: usize,
    /// Rules set to `off`.
    off: usize,
}

impl ExplainLevelCounts {
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
struct ExplainConfiguredRule {
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
