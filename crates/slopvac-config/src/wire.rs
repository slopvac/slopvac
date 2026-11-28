//! Schema-only types for `slopvac.toml`.
//!
//! The reader validates a document with the [`Shape`](crate::schema::Shape) tree, which keeps
//! the file, line and column of every value. These types describe the same document so the
//! generator can write `schemas/v3/config.schema.json`. No code reads a value of them. Each
//! enum list, bound and pattern comes from a constant in [`crate::schema`], the one source
//! both sides read, and `tests/schema_agreement.rs` runs the conformance corpus through the
//! reader and through the schema generated here.

use schemars::generate::Contract;
use schemars::transform::{RecursiveTransform, Transform};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Value, json};
use slopvac_model::{
    NonEmptyString, PackId, PluginId, SchemaVersion, absent_not_null, schema_settings, x_extensions,
};

use crate::MAX_INDEX_EXTRA;
use crate::schema::REL_PATH_PATTERN;
use crate::schema::{
    BUDGET_NONE, DEFAULT_ALLOW, DEFAULT_FRONTMATTER_ENABLED, DEFAULT_GUIDANCE_DEPTH,
    DEFAULT_GUIDANCE_LEVEL, DEFAULT_GUIDANCE_REPORT, DEFAULT_GUIDANCE_REVIEW,
    DEFAULT_GUIDANCE_WRITE, DEFAULT_INDEX_DEFAULT, DEFAULT_MODE, DEFAULT_OFFLINE,
    DEFAULT_UPDATE_CHECK, GUIDANCE_DEPTHS, GUIDANCE_LEVELS, GUIDANCE_REPORT, GUIDANCE_REVIEW,
    GUIDANCE_WRITE, INDEX_SOURCE_MAX, INDEX_SOURCE_PATTERN, LEVELS, LOCALES, MIN_FILES, MIN_STRING,
    MINIMUM, MODES, PROFILES, REF_PATTERN, RULE_KEY_PATTERN, SCORE_MAX, SETTINGS_KEYS, SHIFT_MAX,
    SHIFT_MIN, index_name_pattern,
};

/// The generator for the config schemas: the shared v3 settings, for what a reader accepts.
/// Every member is absent or a real value, never `null`, so each `Option` loses the `null`
/// alternative schemars adds.
fn generator() -> SchemaGenerator {
    schema_settings(Contract::Deserialize)
        .with_transform(RecursiveTransform(absent_not_null))
        .into_generator()
}

/// The schema of `slopvac.toml`, with its `$defs`. `slopvac-schemas` writes it to
/// `schemas/v3/config.schema.json`.
#[must_use]
pub fn config_schema() -> Value {
    let mut generator = generator();
    // Overrides, genre blocks and the root refer to `Settings` through a transform, which
    // cannot register a definition, so register it first.
    generator.subschema_for::<Settings>();
    generator
        .into_root_schema_for::<ConfigDocument>()
        .to_value()
}

/// Settings come from `$defs/Settings` through `allOf`, and the closed set of members is the
/// settings plus the members the type declares: `unevaluatedProperties` states that.
fn over_settings(schema: &mut Schema) {
    schema.remove("additionalProperties");
    schema.insert("allOf".into(), json!([{"$ref": "#/$defs/Settings"}]));
    schema.insert("patternProperties".into(), json!({"^x-": {}}));
    schema.insert("unevaluatedProperties".into(), json!(false));
}

/// As [`over_settings`] for a block that the schema does not declare as an object itself.
fn over_settings_block(schema: &mut Schema) {
    over_settings(schema);
    schema.remove("type");
}

/// An object whose keys are rule or category names, with `$defs/RuleKey` as the name and `V`
/// as the value: `propertyNames` plus `additionalProperties`. schemars writes the key
/// pattern into `patternProperties` for a map type, and the published form is the one here.
fn keyed<V: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "propertyNames": generator.subschema_for::<RuleKey>(),
        "additionalProperties": generator.subschema_for::<V>(),
    })
}

/// Write the two alternatives of an untagged enum, a bare value and a table, as
/// `if` the value is an object, `then` the table, `else` the bare value. The two agree when
/// no value is both, and the starter-config generator reads the `else` branch for its example.
fn scalar_or_table(schema: &mut Schema) {
    let alternatives = match schema.remove("anyOf") {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    let mut scalar = Value::Null;
    let mut table = Value::Null;
    for alternative in alternatives {
        if alternative.get("type").and_then(Value::as_str) == Some("object") {
            table = alternative;
        } else {
            scalar = alternative;
        }
    }
    schema.insert("if".into(), json!({"type": "object"}));
    schema.insert("then".into(), table);
    schema.insert("else".into(), scalar);
}

/// A gitignore-style pattern, matched against the path relative to the directory of the
/// nearest config, the one that applies to the file, even when an extended file holds the
/// pattern. A leading `!` negates. The last matching pattern wins.
#[derive(JsonSchema)]
#[schemars(with = "String", extend("minLength" = MIN_STRING))]
pub(crate) struct Glob;

/// A style or genre name. A bare name resolves against the active plugins and must be
/// unambiguous; `<plugin>:<name>` is exact.
#[derive(JsonSchema)]
#[schemars(with = "String", extend("pattern" = REF_PATTERN))]
pub(crate) struct Ref;

/// The level a rule or category reports at. `off` disables it.
#[derive(JsonSchema)]
#[schemars(with = "String", extend("enum" = LEVELS))]
pub(crate) struct RuleLevel;

/// A rule or category name: qualified `<plugin>:<id>` or a bare name that is unambiguous
/// across the active plugins. Keys of maps in this file are data, so the `x-` convention does
/// not apply to them.
#[derive(JsonSchema)]
#[schemars(with = "String", extend("pattern" = RULE_KEY_PATTERN))]
pub(crate) struct RuleKey;

/// Spelling target. `und` disables the spelling check.
#[derive(JsonSchema)]
#[schemars(with = "String", extend("enum" = LOCALES))]
pub(crate) struct Locale;

/// Document-level gates, evaluated after every finding is collected.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct Thresholds {
    /// Severity-weighted error and warning density per 100 words; errors count 1.0 and
    /// warnings 0.5.
    #[schemars(extend("minimum" = MINIMUM))]
    max_total_per_100_words: Option<f64>,
    #[schemars(extend("minimum" = MINIMUM))]
    max_errors: Option<u32>,
    #[schemars(extend("minimum" = MINIMUM))]
    max_warnings: Option<u32>,
    /// Floor on the 0-100 quality score.
    #[schemars(extend("minimum" = MINIMUM, "maximum" = SCORE_MAX))]
    min_score: Option<f64>,
    /// Ceiling on Unicode em and en dashes in the source.
    #[schemars(extend("minimum" = MINIMUM))]
    max_unicode_dashes: Option<u32>,
}

// The names of the built-in profiles.
#[derive(JsonSchema)]
#[schemars(inline, with = "String", extend("enum" = PROFILES))]
pub(crate) struct ProfileName;

// The table form of a profile.
#[derive(JsonSchema)]
#[schemars(inline, transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct ProfileTable {
    name: Option<ProfileName>,
    /// Steps to move the inherited severity of each rule along suggestion, warning, error.
    /// Inherited means the level from rule definitions, styles and the genre chain. Positive
    /// is stricter. A level that a rules or categories entry sets is final and does not move.
    /// `off` never moves, and results clamp at the ends.
    #[schemars(extend("minimum" = SHIFT_MIN, "maximum" = SHIFT_MAX))]
    severity_shift: Option<i64>,
}

/// A built-in strictness tier, as a name or as a table that adds a severity shift. A profile
/// sets gate thresholds and shifts severities, and nothing else.
#[derive(JsonSchema)]
#[schemars(untagged, transform = scalar_or_table)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the variants and no code builds one"
)]
pub(crate) enum Profile {
    Name(ProfileName),
    Table(ProfileTable),
}

// A density budget: a number, or the string none.
struct Budget;

impl JsonSchema for Budget {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Budget".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"oneOf": [{"type": "number", "minimum": MINIMUM}, {"const": BUDGET_NONE}]})
    }
}

// The table form of the per-category dials.
#[derive(JsonSchema)]
#[schemars(inline, transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct CategoryTable {
    severity: Option<RuleLevel>,
    /// Lowest level that inherited rules in this category may report at. `off` means no
    /// floor, and it cancels a floor set in a lower layer.
    minimum_severity: Option<RuleLevel>,
    /// Density budget; above it the category fails even with no error finding. The string
    /// none clears a budget that a lower layer set, so the category has no budget.
    max_per_100_words: Option<Budget>,
    /// Multiplier on this category's share of the score. 0 makes it informational.
    #[schemars(extend("minimum" = MINIMUM))]
    weight: Option<f64>,
}

/// Per-category dials. A bare level stands for `{ severity = <level> }`.
#[derive(JsonSchema)]
#[schemars(untagged, transform = scalar_or_table)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the variants and no code builds one"
)]
pub(crate) enum CategorySettings {
    Level(RuleLevel),
    Table(CategoryTable),
}

/// Project vocabulary.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct Terms {
    /// Words this project spells or uses its own way. The spelling check and identifier
    /// checks skip them.
    allow: Option<Vec<NonEmptyString>>,
    /// Terms files, relative to the config file.
    files: Option<Vec<NonEmptyString>>,
    /// Word blocklist files, relative to the config file. Each entry names a word, a part of
    /// speech and a reason.
    blocklists: Option<Vec<NonEmptyString>>,
}

// Category names to their dials.
struct CategoriesMap;

impl JsonSchema for CategoriesMap {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "CategoriesMap".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        keyed::<CategorySettings>(generator)
    }
}

// Rule names to levels.
struct RulesMap;

impl JsonSchema for RulesMap {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "RulesMap".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        keyed::<RuleLevel>(generator)
    }
}

/// Settings that a layer below the project config, an override, a genre block and front
/// matter can all set. Each layer patches the one below field by field. Maps merge by key.
/// Lists and scalars replace.
#[derive(JsonSchema)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct Settings {
    profile: Option<Profile>,
    locale: Option<Locale>,
    thresholds: Option<Thresholds>,
    categories: Option<CategoriesMap>,
    /// Rule levels by rule name.
    rules: Option<RulesMap>,
    terms: Option<Terms>,
}

/// A data plugin or code plugin the project depends on. The lockfile pins the digest and
/// records the kind.
#[derive(JsonSchema)]
#[schemars(rename = "Source", transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct PluginSource {
    id: PluginId,
    /// An `oci://` reference, a `git+https://` URL with a commit, or a path relative to the
    /// config file.
    #[schemars(extend("minLength" = MIN_STRING))]
    source: String,
    #[schemars(extend("minLength" = MIN_STRING))]
    version: Option<String>,
}

/// The name of an extra index: a plugin-id-shaped slug. `core` is reserved for the built-in
/// plugin and `slopvac` names the default index.
#[derive(JsonSchema)]
#[schemars(with = "String", extend("pattern" = index_name_pattern()))]
pub(crate) struct IndexName;

/// Where to fetch an extra index: an `oci://` reference or an `https://` URL. The URL carries
/// no user-info. `http://`, `git+https://` and path sources are invalid.
#[derive(JsonSchema)]
#[schemars(
    with = "String",
    extend("maxLength" = INDEX_SOURCE_MAX, "pattern" = INDEX_SOURCE_PATTERN)
)]
pub(crate) struct IndexExtraSource;

/// An extra plugin index. Search results say `listed by <name>`. Its publisher keys are for
/// display only.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct IndexExtra {
    name: IndexName,
    source: IndexExtraSource,
}

/// Which plugin indexes `slopvac plugins search` reads. Project-scoped: no user config,
/// environment variable, front matter or override sets it. Naming an index grants it no
/// trust. Semantics: docs/contract/plugin-index.md, sections 5 and 6.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct IndexConfig {
    /// Read the signed first-party index, named `slopvac`.
    #[schemars(extend("default" = DEFAULT_INDEX_DEFAULT))]
    default: Option<bool>,
    /// Extra unsigned indexes, at most 16, read in this order after the default index. Each
    /// `name` appears once; the config validator checks that, because JSON Schema cannot.
    #[schemars(extend("maxItems" = MAX_INDEX_EXTRA))]
    extra: Option<Vec<IndexExtra>>,
}

/// A glob-scoped patch. Every matching block applies in file order and the last writer of a
/// field wins.
#[derive(JsonSchema)]
#[schemars(transform = over_settings_block)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct Override {
    #[schemars(extend("minItems" = MIN_FILES))]
    files: Vec<Glob>,
}

/// Assigns a genre to the files it matches and patches their settings. Genre definitions live
/// in genre files.
#[derive(JsonSchema)]
#[schemars(transform = over_settings_block)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct GenreBlock {
    genre: Ref,
    #[schemars(extend("minItems" = MIN_FILES))]
    files: Vec<Glob>,
}

// The input surfaces.
#[derive(JsonSchema)]
#[schemars(inline, with = "String", extend("enum" = MODES))]
struct ModeName;

// The detail levels of the rendered prime topics.
#[derive(JsonSchema)]
#[schemars(inline, with = "String", extend("enum" = GUIDANCE_LEVELS))]
struct GuidanceLevel;

// The values of guidance.write.
#[derive(JsonSchema)]
#[schemars(inline, with = "String", extend("enum" = GUIDANCE_WRITE))]
struct GuidanceWrite;

// The values of guidance.review.
#[derive(JsonSchema)]
#[schemars(inline, with = "String", extend("enum" = GUIDANCE_REVIEW))]
struct GuidanceReview;

// The values of guidance.review_depth.
#[derive(JsonSchema)]
#[schemars(inline, with = "String", extend("enum" = GUIDANCE_DEPTHS))]
struct GuidanceDepth;

// The values of guidance.report.
#[derive(JsonSchema)]
#[schemars(inline, with = "String", extend("enum" = GUIDANCE_REPORT))]
struct GuidanceReport;

/// Agent steering. Project-wide: overrides, genre blocks and front matter cannot set it.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct Guidance {
    /// Detail of the rendered `prime` topics.
    #[schemars(extend("default" = DEFAULT_GUIDANCE_LEVEL))]
    level: Option<GuidanceLevel>,
    #[schemars(extend("default" = DEFAULT_GUIDANCE_WRITE))]
    write: Option<GuidanceWrite>,
    #[schemars(extend("default" = DEFAULT_GUIDANCE_REVIEW))]
    review: Option<GuidanceReview>,
    #[schemars(extend("default" = DEFAULT_GUIDANCE_DEPTH))]
    review_depth: Option<GuidanceDepth>,
    #[schemars(extend("default" = DEFAULT_GUIDANCE_REPORT))]
    report: Option<GuidanceReport>,
}

// A field that a document block may set.
#[derive(JsonSchema)]
#[schemars(inline, with = "String", extend("enum" = SETTINGS_KEYS))]
struct SettingKey;

/// Which fields of a document's `slopvac:` block take effect. A block field outside the
/// allowed set is an error at its line and column.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct FrontmatterPolicy {
    /// When false, `slopvac:` is plain data and is not read.
    #[schemars(extend("default" = DEFAULT_FRONTMATTER_ENABLED))]
    enabled: Option<bool>,
    #[schemars(extend("uniqueItems" = true, "default" = DEFAULT_ALLOW))]
    allow: Option<Vec<SettingKey>>,
}

/// Network policy. `offline = true` is sticky: no other source can unset it, so `false` only
/// restates the default. See docs/contract/policies.md.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct Network {
    #[schemars(extend("default" = DEFAULT_OFFLINE))]
    offline: Option<bool>,
    #[schemars(extend("default" = DEFAULT_UPDATE_CHECK))]
    update_check: Option<bool>,
}

/// Agent review. The provider is a trusted plugin; it stays idle during lint.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct Review {
    /// Plugin id of the model provider.
    provider: Option<PackId>,
    /// Model name the reviewer must report.
    #[schemars(extend("minLength" = MIN_STRING))]
    model: Option<String>,
    /// Model name of the author. The review refuses a reviewer with the same name.
    #[schemars(extend("minLength" = MIN_STRING))]
    author_model: Option<String>,
}

/// Project config. Unknown keys are errors; keys that start with `x-` are ignored. Semantics:
/// docs/contract/config.md.
#[derive(JsonSchema)]
#[schemars(transform = over_settings)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct ConfigDocument {
    schema_version: SchemaVersion,
    /// Input surface.
    #[schemars(extend("default" = DEFAULT_MODE))]
    mode: Option<ModeName>,
    /// Config files merged below this one, lowest first. A path relative to this file or
    /// `<plugin>:<name>` for a plugin config.
    extends: Option<Vec<NonEmptyString>>,
    /// Ordered style presets. The last listed wins.
    styles: Option<Vec<Ref>>,
    /// Files never linted.
    exclude: Option<Vec<Glob>>,
    /// Data plugins and code plugins the project uses, in one list. Each id appears once, and
    /// `core` is reserved for the built-in plugin. A code plugin also needs
    /// `slopvac plugins trust`; one that is not trusted never starts.
    plugins: Option<Vec<PluginSource>>,
    index: Option<IndexConfig>,
    genres: Option<Vec<GenreBlock>>,
    frontmatter: Option<FrontmatterPolicy>,
    guidance: Option<Guidance>,
    overrides: Option<Vec<Override>>,
    network: Option<Network>,
    review: Option<Review>,
}

/// Replace a field schema with a reference to one setting of `Settings` in the config schema,
/// so the front-matter block and a config table share one definition of each setting.
struct SettingsField(&'static str);

impl Transform for SettingsField {
    fn transform(&mut self, schema: &mut Schema) {
        *schema = json_schema!({"$ref": format!("#/$defs/Settings/properties/{}", self.0)});
    }
}

// Words this document uses its own way. Front matter takes no terms files.
#[derive(JsonSchema)]
#[schemars(inline, transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct BlockTerms {
    allow: Option<Vec<NonEmptyString>>,
}

/// Per-document settings. The layer sits above matching overrides and below inline directives.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct Block {
    #[expect(
        clippy::doc_markdown,
        reason = "the text is the published description, which names the member without backticks"
    )]
    /// Version of this block's shape. Same number as the config schema_version.
    v: SchemaVersion,
    genre: Option<Ref>,
    #[schemars(with = "Option<String>", transform = SettingsField("profile"))]
    profile: Option<String>,
    #[schemars(with = "Option<String>", transform = SettingsField("locale"))]
    locale: Option<String>,
    #[schemars(with = "Option<String>", transform = SettingsField("thresholds"))]
    thresholds: Option<String>,
    #[schemars(with = "Option<String>", transform = SettingsField("categories"))]
    categories: Option<String>,
    #[schemars(with = "Option<String>", transform = SettingsField("rules"))]
    rules: Option<String>,
    /// Words this document uses its own way.
    terms: Option<BlockTerms>,
}

/// A document's front matter mapping, after YAML, TOML or JSON parsing. Only the `slopvac` key
/// belongs to slopvac; every other key is the document's own and is not validated. Unknown keys
/// inside `slopvac` are errors, and keys that start with `x-` are ignored. The project's
/// `[frontmatter] allow` list narrows which fields take effect. Semantics:
/// docs/contract/config.md.
#[derive(JsonSchema)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct FrontMatterDocument {
    slopvac: Option<Block>,
}

/// The schema of the `slopvac:` front-matter block. The references to `Settings` stay local
/// here, and `slopvac-schemas` points them at the config schema.
#[must_use]
pub fn frontmatter_schema() -> Value {
    generator()
        .into_root_schema_for::<FrontMatterDocument>()
        .to_value()
}

/// A path relative to this file: no leading slash or drive letter, no backslash and no .. segment.
#[derive(JsonSchema)]
#[schemars(with = "String", extend("minLength" = MIN_STRING, "pattern" = REL_PATH_PATTERN))]
pub(crate) struct RelPath;

/// Terms the style brings.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct StyleTerms {
    /// Words the style lets through the spelling check. A list replaces.
    allow: Option<Vec<NonEmptyString>>,
    /// Blocklist files, relative to this style file.
    blocklists: Option<Vec<RelPath>>,
}

/// An ordered preset of category and rule levels and of terms. A config lists styles in order
/// and the last one wins. A style may extend other styles, which apply first. A style declares
/// the styles it cannot combine with, and the host reports a conflict when both are active.
/// Semantics are specified in docs/contract/plugins.md. The style name is the file name without
/// its extension, qualified by the plugin id. Maps merge by key and lists replace, as in
/// config.schema.json. Category entries, rule levels and names are the definitions in
/// config.schema.json.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct StyleDocument {
    schema_version: SchemaVersion,
    description: Option<NonEmptyString>,
    /// Styles applied before this one, lowest first.
    #[schemars(extend("uniqueItems" = true))]
    extends: Option<Vec<Ref>>,
    /// Styles that must not be active together with this one.
    #[schemars(extend("uniqueItems" = true))]
    conflicts: Option<Vec<Ref>>,
    /// Category dials, keyed by category name. A bare level sets the severity of every rule in
    /// the category. Keys are data, so the x- convention does not apply: an x- key is a rule or
    /// category name and takes a level.
    categories: Option<CategoriesMap>,
    /// Rule levels, keyed by rule name. A rule entry wins over the category entry. Keys are
    /// data, so the x- convention does not apply: an x- key is a rule or category name and
    /// takes a level.
    rules: Option<RulesMap>,
    terms: Option<StyleTerms>,
}

/// The schema of a style preset file, with its `$defs`. The references to config definitions
/// stay local here, and `slopvac-schemas` points them at the config schema.
/// `slopvac-schemas` writes it to `schemas/v3/style.schema.json`.
#[must_use]
pub fn style_schema() -> Value {
    generator()
        .into_root_schema_for::<StyleDocument>()
        .to_value()
}
