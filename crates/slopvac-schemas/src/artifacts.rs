use schemars::generate::Contract;
use schemars::{JsonSchema, SchemaGenerator};
use serde_json::{Map, Value};
use slopvac_model::{
    ApiVersion, ConfigSource, ContextTag, Edit, LineCol, PackId, PluginId, Report, RuleId,
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
        file: "config.schema.json",
        title: "slopvac v3 configuration (slopvac.toml)",
        description: None,
        imports: &["common.schema.json"],
        inline: &[],
        keep: &[],
        build: config,
    },
    Artifact {
        file: "frontmatter.schema.json",
        title: "slopvac v3 front matter",
        description: None,
        imports: &["common.schema.json", "config.schema.json"],
        inline: &[],
        keep: &[],
        build: frontmatter,
    },
    Artifact {
        file: "terms.schema.json",
        title: "slopvac v3 terms file",
        description: Some(
            "A list of project terms: product names, acronyms and other words that rules must \
             not flag or must spell a fixed way. The project file is .slopvac/terms.toml, and \
             plugins ship more. Semantics are specified in docs/contract/plugins.md.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        keep: &[],
        build: terms,
    },
    Artifact {
        file: "blocklist.schema.json",
        title: "slopvac v3 word blocklist",
        description: Some(
            "A list of words a project refuses, each with a part of speech and a reason. The \
             opt-in vocabulary check reads it; it is off unless `terms.blocklists` or \
             `[vocabulary] path` names a file. The file may be TOML, JSON or YAML (`.toml`, \
             `.json`, `.yml` or `.yaml`); the file suffix picks the format. Semantics are \
             specified in docs/contract/plugins.md.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        keep: &[],
        build: blocklist,
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
    Artifact {
        file: "plugin-protocol.schema.json",
        title: "slopvac plugin protocol v1",
        description: Some(
            "One JSON-RPC 2.0 message of plugin protocol v1, as carried in the body of a \
             Content-Length frame. The root validates a whole message: a request or \
             notification is dispatched on method, a response is checked as a success or an \
             error. Result shapes are separate $defs because a response names no method; \
             validate one against the def of the method it answers. No object here sets \
             additionalProperties false: a reader ignores unknown fields, and keys with the \
             prefix x- never carry meaning. Semantics are in \
             docs/contract/plugin-protocol.md. Owner: slopvac-8l4.2.6.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        // The result definitions: a response names no method, so no message refers to them.
        keep: &[
            "BinaryExtractResult",
            "CompleteResult",
            "ExtractResult",
            "FixResult",
            "InitializeResult",
            "PluginDeclaration",
            "RecipeResult",
            "RenderResult",
            "ScoreResult",
            "TagResult",
        ],
        build: plugin_protocol,
    },
    Artifact {
        file: "claims.schema.json",
        title: "slopvac v3 claims report",
        description: Some(
            "The document that `slopvac claims` prints by default: the sentences a revision \
             changed or added, and the original sentences it dropped, with the literals and \
             negations that differ. It is public output.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        keep: &[],
        build: claims_report,
    },
    Artifact {
        file: "review-plan.schema.json",
        title: "slopvac v3 review plan",
        description: Some(
            "The document that `slopvac review` prints: the review type and why, the change \
             stats, the scope budget, the exact payload a provider receives, the model notices \
             and, after a provider call, the provider's report. It is public output.",
        ),
        imports: &["common.schema.json", "lint-report.schema.json"],
        inline: &[],
        keep: &[],
        build: review_plan,
    },
    Artifact {
        file: "explain-config.schema.json",
        title: "slopvac v3 explain-config",
        description: Some(
            "The document that `slopvac lint --explain-config --format json` prints: for each \
             input, the resolved config with the layer and location that set each value, the \
             rules a layer gave a level and whether front matter took part. Paths are relative \
             to the working directory, or `<external>/<name>` outside it (policies.md, section \
             1.7). It carries no document text and no front matter value. Semantics are \
             specified in docs/contract/config.md, section 1.4.1.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        keep: &[],
        build: explain_config,
    },
    Artifact {
        file: "version.schema.json",
        title: "slopvac v3 version",
        description: Some(
            "The document that `slopvac --version --json` prints: the binary version and how it \
             was installed, the `schema_version` numbers the binary reads or writes for each \
             versioned artifact, the `slopvac_api` numbers it supports, and the update notice \
             when a newer release is known. `slopvac self update` reads the same document from \
             the installed executable, and ignores the members it does not know. Unknown keys \
             are errors, and keys that start with `x-` are ignored. Semantics are specified in \
             docs/contract/policies.md, section 3.2.",
        ),
        imports: &["common.schema.json"],
        inline: &[],
        keep: &[],
        build: version_document,
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

/// One message of the plugin protocol. Both sides read and write messages, so the schema holds
/// what a reader accepts.
fn plugin_protocol() -> Value {
    schema_settings(Contract::Deserialize)
        .into_generator()
        .into_root_schema_for::<slopvac_plugins::ProtocolMessage>()
        .to_value()
}

/// The producer contract: what `slopvac claims` serializes.
fn claims_report() -> Value {
    schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<slopvac_agents::claims::Report>()
        .to_value()
}

/// The producer contract: what `slopvac review` serializes.
fn review_plan() -> Value {
    schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<slopvac_agents::review::Plan>()
        .to_value()
}

/// The producer contract: what `slopvac lint --explain-config --format json` serializes.
fn explain_config() -> Value {
    schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<slopvac_cli::explain_config::Document>()
        .to_value()
}

/// The producer contract: what `slopvac --version --json` serializes.
fn version_document() -> Value {
    schema_settings(Contract::Serialize)
        .into_generator()
        .into_root_schema_for::<slopvac_update::document::Document>()
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

/// Semantic version 2.0.0 without build metadata. Numeric prerelease identifiers have no
/// leading zeros.
#[derive(JsonSchema)]
// Schema only, like `PluginId`. The plugin manifest and lock readers check the value
// themselves. The definition lives here, not in the manifest schema, because the lock and
// the index are read and written by slopvac, and a definition they share must sit in a file
// that follows the output rule.
#[schemars(with = "String", extend("pattern" = r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?(?![\s\S])"))]
pub(crate) struct Semver;

/// The project config, which the reader accepts as input. The types are schema-only
/// (`slopvac_config::config_schema`), and the reader validates with its own shape tree.
fn config() -> Value {
    slopvac_config::config_schema()
}

/// The front-matter block, which the reader accepts as input. Its settings refer to the
/// config schema, which `slopvac-schemas` links.
fn frontmatter() -> Value {
    slopvac_config::frontmatter_schema()
}

/// The terms file, which the reader accepts as input. The types are schema-only
/// (`slopvac_terms::terms_schema`), and the reader validates the file itself.
fn terms() -> Value {
    slopvac_terms::terms_schema()
}

/// The word blocklist, which the reader accepts as input, in the same way.
fn blocklist() -> Value {
    slopvac_terms::blocklist_schema()
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
