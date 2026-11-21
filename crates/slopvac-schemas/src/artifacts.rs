use schemars::generate::Contract;
use schemars::{JsonSchema, SchemaGenerator};
use serde_json::{Map, Value};
use slopvac_model::{
    ApiVersion, ConfigSource, ContextTag, Edit, LineCol, PackId, Report, RuleId, RuleProvenance,
    SchemaVersion, Severity, Span, schema_settings,
};
use slopvac_rules::{Category, RuleFile, schema_generator};

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
];

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
#[schemars(with = "String", extend("pattern" = r"^(?!core(?![\s\S]))[a-z][a-z0-9-]*(?![\s\S])"))]
struct PluginId;

/// Semantic version 2.0.0 without build metadata. Numeric prerelease identifiers have no
/// leading zeros.
#[derive(JsonSchema)]
// Schema only, like `PluginId`. The plugin manifest and lock readers check the value
// themselves. The definition lives here, not in the manifest schema, because the lock and
// the index are read and written by slopvac, and a definition they share must sit in a file
// that follows the output rule.
#[schemars(with = "String", extend("pattern" = r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?(?![\s\S])"))]
struct Semver;

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
