use schemars::generate::Contract;
use schemars::{JsonSchema, SchemaGenerator};
use serde_json::{Map, Value};
use slopvac_model::{
    ApiVersion, ConfigSource, ContextTag, Edit, LineCol, PackId, RESERVED_ID, Report, RuleId,
    RuleProvenance, SchemaVersion, Severity, Span, schema_settings,
};
use slopvac_rules::{Category, RuleFile, schema_generator};

use crate::plugin_files::{PackIndex, PluginsLock};

/// One generated file in `schemas/v3`.
#[derive(Debug, Clone, Copy)]
pub struct Artifact {
    /// File name, such as `common.schema.json`. The `$id` is the publishing base plus this.
    pub file: &'static str,
    /// The `title` keyword.
    pub title: &'static str,
    /// The `description` keyword, when the root type's doc comment does not supply it.
    pub description: Option<&'static str>,
    /// Files whose `$defs` this one references. Each shared definition is published once, by
    /// the imported file that defines it, and referenced from here.
    pub imports: &'static [&'static str],
    /// Local definitions written in place instead of under `$defs`, because another file
    /// publishes a definition of the same name. A `$defs` name means one thing across
    /// `schemas/v3`.
    pub inline: &'static [&'static str],
    /// Build the schema: a root type's schema with its `$defs`, or `$defs` alone.
    pub build: fn() -> Value,
}

/// Every generated file, each listed after the files it imports.
pub const ARTIFACTS: &[Artifact] = &[
    Artifact {
        file: "common.schema.json",
        title: "slopvac v3 shared definitions",
        description: Some("Shared $defs referenced by every v3 artifact schema."),
        imports: &[],
        inline: &[],
        build: common,
    },
    Artifact {
        file: "rule.schema.json",
        title: "slopvac v3 rule",
        description: Some(
            "One check. The root validates a standalone rule file and requires schema_version. \
             A category file embeds the same shape through $defs/Rule, where schema_version is \
             absent. Semantics are specified in docs/contract/rules.md.",
        ),
        imports: &["common.schema.json"],
        inline: &["AiSignal"],
        build: rule,
    },
    Artifact {
        file: "category.schema.json",
        title: "slopvac v3 category",
        description: None,
        imports: &["common.schema.json", "rule.schema.json"],
        inline: &["CategoryId", "Density"],
        build: category,
    },
    Artifact {
        file: "lint-report.schema.json",
        title: "slopvac v3 lint report",
        description: None,
        imports: &["common.schema.json"],
        inline: &["Genre"],
        build: lint_report,
    },
    Artifact {
        file: "plugin-manifest.schema.json",
        title: "slopvac v3 plugin manifest",
        description: Some(
            "The slopvac-plugin.toml file at the root of a plugin. A data plugin holds rules, \
             styles, genres, terms, taggers and prime snippets, and no code. A code plugin adds \
             one [plugin] table with the command to run and the embedded plugin declaration. The \
             manifest of a code plugin is part of the content hash that `slopvac plugins trust` \
             records. Semantics are specified in docs/contract/plugins.md.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        build: manifest,
    },
    Artifact {
        file: "doctor-report.schema.json",
        title: "slopvac v3 doctor report",
        description: Some(
            "The document that `slopvac doctor --format json` prints. It reports the binary, \
             update state, configuration sources, locked plugins, plugin trust and the per-user \
             directories. Doctor reads local files only and never changes them. Paths are \
             relative to the working directory, or `<external>/<name>` outside it (policies.md, \
             section 1.7). Unknown keys are errors, and keys that start with `x-` are ignored. \
             Semantics are specified in docs/contract/doctor.md.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        build: doctor_report,
    },
    Artifact {
        file: "plugin-index.schema.json",
        title: "slopvac v3 plugin index",
        description: Some(
            "The plugin index document that `slopvac plugins search` reads. A scheduled job \
             rebuilds it from plugins that follow the naming convention. The index lists plugins \
             for discovery and nothing more: it is not an authority for keys, and `publisher_key` \
             is display-only (threat-model.md, section 5.2). A reader refuses a document over 32 \
             MiB before it parses. Unknown keys are errors, and keys that start with `x-` are \
             ignored. Semantics are specified in docs/contract/plugin-index.md.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        build: plugin_index,
    },
    Artifact {
        file: "plugins-lock.schema.json",
        title: "slopvac v3 plugins lockfile",
        description: Some(
            "The .slopvac/plugins.lock file. `slopvac plugins sync` writes it and a run reads \
             it. It pins each data plugin and code plugin to a version, a source and a content \
             digest, so a run on another machine loads the same bytes. A digest mismatch makes \
             the run fail with exit 2. The lockfile records no trust: trust is a separate \
             per-user record. The built-in core plugin is compiled into the binary and never \
             appears here. Semantics are specified in docs/contract/plugins.md.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        build: plugins_lock,
    },
];

/// The plugin index. Both the index and the lockfile are read and written by slopvac, so the
/// output rule applies to them.
fn plugin_index() -> Value {
    schema_settings(Contract::Deserialize)
        .into_generator()
        .into_root_schema_for::<PackIndex>()
        .to_value()
}

fn plugins_lock() -> Value {
    schema_settings(Contract::Deserialize)
        .into_generator()
        .into_root_schema_for::<PluginsLock>()
        .to_value()
}

/// The producer contract: what the doctor report serializes.
fn doctor_report() -> Value {
    schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<slopvac_cli::doctor::Report>()
        .to_value()
}

fn manifest() -> Value {
    schema_settings(Contract::Deserialize)
        .into_generator()
        .into_root_schema_for::<slopvac_packs::Manifest>()
        .to_value()
}

fn common() -> Value {
    let mut generator = schema_settings(Contract::Deserialize).into_generator();
    let defs = [
        define::<Span>,
        define::<LineCol>,
        define::<Edit>,
        define::<PackId>,
        define::<PluginId>,
        define::<Semver>,
        define::<RuleId>,
        define::<Severity>,
        define::<RuleProvenance>,
        define::<ConfigSource>,
        define::<ContextTag>,
        define::<SchemaVersion>,
        define::<ApiVersion>,
    ];
    for define in defs {
        define(&mut generator);
    }
    let mut root = Map::new();
    root.insert(
        "$defs".into(),
        Value::Object(generator.take_definitions(true)),
    );
    Value::Object(root)
}

fn define<T: JsonSchema>(generator: &mut SchemaGenerator) {
    generator.subschema_for::<T>();
}

/// Identifier of a plugin, or of the namespace a publisher signs for. Same character set as
/// `PackId`. `core` is reserved for the built-in plugin that is compiled into the binary, so
/// no manifest, lock entry, index entry or config entry may use it as an id or a namespace.
/// Semantics: docs/contract/plugins.md, section 9.
#[derive(JsonSchema)]
// Schema only. No crate has a Rust type for it yet: the plugin lock and index readers and the
// config validator check the value themselves. The generator owns this definition until a model
// type replaces it.
#[schemars(with = "String", extend("pattern" = plugin_id_pattern()))]
pub(crate) struct PluginId;

/// Semantic version 2.0.0 without build metadata. Numeric prerelease identifiers have no
/// leading zeros.
#[derive(JsonSchema)]
// Schema only, like `PluginId`. The plugin manifest and lock readers check the value
// themselves. The definition lives here, not in the manifest schema, because the lock and
// the index are read and written by slopvac, and a definition they share must sit in a file
// that follows the output rule.
#[schemars(with = "String", extend("pattern" = r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?(?![\s\S])"))]
pub(crate) struct Semver;

/// Slug pattern that refuses the reserved id.
fn plugin_id_pattern() -> String {
    format!(r"^(?!{RESERVED_ID}(?![\s\S]))[a-z][a-z0-9-]*(?![\s\S])")
}

fn rule() -> Value {
    schema_generator()
        .into_root_schema_for::<RuleFile>()
        .to_value()
}

fn category() -> Value {
    schema_generator()
        .into_root_schema_for::<Category>()
        .to_value()
}

/// The producer contract: what `Report` serializes, so a member the producer always writes
/// stays required even where `Deserialize` would accept it absent.
fn lint_report() -> Value {
    schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<Report>()
        .to_value()
}
