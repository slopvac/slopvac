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
    /// Local definitions that stay under `$defs` although nothing in the file refers to them,
    /// because another file's schema does. The build must generate each one.
    pub keep: &'static [&'static str],
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
        keep: &[],
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
        keep: &[],
        build: rule,
    },
    Artifact {
        file: "category.schema.json",
        title: "slopvac v3 category",
        description: None,
        imports: &["common.schema.json", "rule.schema.json"],
        inline: &["CategoryId", "Density"],
        keep: &[],
        build: category,
    },
    Artifact {
        file: "tagger.schema.json",
        title: "slopvac v3 declarative tagger",
        description: Some(
            "A context tagger written as data. It marks text units with a context tag that \
             rules can gate on (rule contexts only and skip). It runs without code and is \
             deterministic. A tag carries a span, a confidence and evidence, like the tags a \
             code tagger returns over the plugin protocol. Semantics are specified in \
             docs/contract/plugins.md. A tagger has either `match` entries, which read the text \
             of one unit, or a `when` object, which reads document structure.",
        ),
        imports: &["common.schema.json", "rule.schema.json"],
        inline: &[],
        keep: &[],
        build: tagger,
    },
    Artifact {
        file: "lint-report.schema.json",
        title: "slopvac v3 lint report",
        description: None,
        imports: &["common.schema.json"],
        inline: &["Genre"],
        keep: &["CategoryId"],
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
        keep: &[],
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
        keep: &[],
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
        keep: &[],
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
             appears here. Semantics are specified in docs/contract/plugins.md. The reader also \
             refuses a file over 1 MiB, and an id, a dependency id or a version over 4096 bytes.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        keep: &[],
        build: plugins_lock,
    },
    Artifact {
        file: "plugin-protocol-codes.schema.json",
        title: "slopvac v3 plugin protocol codes",
        description: Some(
            "The two error-code tables of plugin-protocol.md section 10, as the data file \
             docs/reference/plugin-protocol-codes.json. The docs site reads this file, not the \
             Markdown. The host's code table in slopvac-plugins is the source, and the file is \
             generated from it.",
        ),
        imports: &[],
        inline: &[],
        keep: &[],
        build: protocol_codes,
    },
];

/// The producer contract: what the generated reference file holds.
fn protocol_codes() -> Value {
    schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<slopvac_plugins::ProtocolCodes>()
        .to_value()
}

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

/// A declarative tagger file, which the reader accepts as input.
fn tagger() -> Value {
    schema_generator()
        .into_root_schema_for::<slopvac_context::TaggerSpec>()
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
    let mut root = schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<Report>()
        .to_value();
    // No report member is a bare `CategoryId` any more, but the rules output refers to this
    // definition, so the file keeps it (see `Artifact::keep`).
    let mut bare = schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<slopvac_model::CategoryId>()
        .to_value();
    if let Some(object) = bare.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
    }
    if let Some(defs) = root.get_mut("$defs").and_then(Value::as_object_mut) {
        defs.insert("CategoryId".to_owned(), bare);
    }
    root
}
