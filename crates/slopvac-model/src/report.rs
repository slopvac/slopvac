use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::edit::Edit;
use crate::ids::{ContextTag, PackId, RuleId, SchemaVersion, Severity};
use crate::span::Span;

/// Optional producer fields are omitted when unset, never written as `null`; drop the
/// `null` alternative schemars adds for `Option` so the schema says the same.
fn absent_not_null(schema: &mut Schema) {
    if let Some(Value::Array(types)) = schema.get_mut("type") {
        types.retain(|t| t != "null");
        if types.len() == 1 {
            let only = types.remove(0);
            schema.insert("type".into(), only);
        }
    }
    if let Some(Value::Array(any)) = schema.get_mut("anyOf") {
        any.retain(|alt| alt.get("type") != Some(&Value::String("null".into())));
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

/// `budget` is always written, as a non-negative number or `null`.
fn nullable_budget(_: &mut schemars::SchemaGenerator) -> Schema {
    schemars::json_schema!({ "type": ["number", "null"], "minimum": 0 })
}

/// How strongly a finding indicates machine-written text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AiSignal {
    /// Strong signal.
    Strong,
    /// Weak signal.
    Weak,
    /// No signal.
    None,
}

/// Severity of a [`Notice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    /// Informational.
    Info,
    /// Needs attention.
    Warning,
}

/// A stable, machine-readable fact with a human message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct Notice {
    /// Stable kebab-case identifier.
    #[schemars(regex(pattern = "^[a-z][a-z0-9]*(-[a-z0-9]+)*$"))]
    pub code: String,
    /// Level.
    pub level: NoticeLevel,
    /// Human message.
    #[schemars(length(min = 1))]
    pub message: String,
    /// File the notice concerns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    pub path: Option<String>,
}

/// Where a rule came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct RuleProvenance {
    /// Owning pack.
    pub pack: PackId,
    /// Pack version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    pub version: Option<String>,
    /// Pack source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    pub source: Option<String>,
}

/// Config layer and location a setting came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct ConfigSource {
    /// Layer name.
    pub layer: String,
    /// File.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    pub file: Option<String>,
    /// One-based line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    #[schemars(range(min = 1))]
    pub line: Option<usize>,
    /// One-based column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    #[schemars(range(min = 1))]
    pub column: Option<usize>,
}

/// A machine-applicable correction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct Fix {
    /// Non-overlapping edits in original offsets.
    #[schemars(length(min = 1))]
    pub edits: Vec<Edit>,
}

/// One violation at one position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}, "dependentRequired" = {"span": ["end_line", "end_column"], "end_line": ["end_column", "span"], "end_column": ["end_line", "span"]}))]
pub struct Finding {
    /// Qualified rule id.
    pub rule_id: RuleId,
    /// Owning pack.
    pub pack: PackId,
    /// Category id.
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub category: String,
    /// Severity.
    pub severity: Severity,
    /// Message.
    #[schemars(length(min = 1))]
    pub message: String,
    /// Byte range; absent for whole-document findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    pub span: Option<Span>,
    /// Start line.
    #[schemars(range(min = 1))]
    pub line: usize,
    /// Start column.
    #[schemars(range(min = 1))]
    pub column: usize,
    /// End line; present with `span`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    #[schemars(range(min = 1))]
    pub end_line: Option<usize>,
    /// End column; present with `span`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    #[schemars(range(min = 1))]
    pub end_column: Option<usize>,
    /// Text contexts the match sits in.
    #[schemars(extend("uniqueItems" = true))]
    pub contexts: Vec<ContextTag>,
    /// AI signal.
    pub ai_signal: AiSignal,
    /// Matched source text.
    pub matched_text: String,
    /// Suggested fix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    pub fix: Option<Fix>,
    /// Rule provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = absent_not_null)]
    pub provenance: Option<RuleProvenance>,
}

/// Counts for one reporting axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct AiRegisterSummary {
    /// Findings.
    pub findings: usize,
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Findings per 100 words.
    #[schemars(range(min = 0))]
    pub per_100_words: f64,
}

/// Strong and weak AI-signal summaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct AiRegister {
    /// Strong signal.
    pub strong: AiRegisterSummary,
    /// Weak signal.
    pub weak: AiRegisterSummary,
}

/// One category rolled up across every document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct CategorySummary {
    /// Category id.
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub category: String,
    /// Findings.
    pub findings: usize,
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Suggestions.
    pub suggestions: usize,
    /// Findings per 100 words.
    #[schemars(range(min = 0))]
    pub per_100_words: f64,
    /// Score, 0 to 100.
    #[schemars(range(min = 0, max = 100))]
    pub score: f64,
}

/// Every document rolled into one result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct RunSummary {
    /// Documents linted.
    pub documents: usize,
    /// Total words.
    pub words: usize,
    /// Findings.
    pub findings: usize,
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Suggestions.
    pub suggestions: usize,
    /// Findings per 100 words.
    #[schemars(range(min = 0))]
    pub per_100_words: f64,
    /// Score, 0 to 100.
    #[schemars(range(min = 0, max = 100))]
    pub score: f64,
    /// Whether every document passed.
    pub passed: bool,
    /// Per-category rollup.
    pub categories: Vec<CategorySummary>,
    /// AI register.
    pub ai_register: AiRegister,
    /// Prose axis.
    pub prose: AiRegisterSummary,
}

/// Where a genre choice came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GenreSource {
    /// Config, command line or front matter.
    Explicit,
    /// Genre signals.
    Detected,
    /// Fallback.
    Default,
}

/// One signal that led to the genre choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct GenreReason {
    /// Kebab-case signal name.
    #[schemars(regex(pattern = "^[a-z][a-z0-9]*(-[a-z0-9]+)*$"))]
    pub signal: String,
    /// Detail.
    pub detail: String,
}

/// The genre applied to a document and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct GenreResult {
    /// Genre id.
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub id: String,
    /// Source of the choice.
    pub source: GenreSource,
    /// Reasons.
    pub reasons: Vec<GenreReason>,
}

/// Per-category result for one document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct CategoryScore {
    /// Category id.
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub category: String,
    /// Findings.
    pub findings: usize,
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Suggestions.
    pub suggestions: usize,
    /// Findings per 100 words.
    #[schemars(range(min = 0))]
    pub per_100_words: f64,
    /// Errors and warnings per 100 words.
    #[schemars(range(min = 0))]
    pub gating_per_100_words: f64,
    /// Gating limit, or null.
    #[schemars(schema_with = "nullable_budget")]
    pub budget: Option<f64>,
    /// Score, 0 to 100.
    #[schemars(range(min = 0, max = 100))]
    pub score: f64,
    /// Whether the budget is exceeded.
    pub over_budget: bool,
}

/// Pass or fail result for one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}, "allOf" = [{"if": {"properties": {"passed": {"const": true}}}, "then": {"properties": {"reasons": {"maxItems": 0}}}}, {"if": {"properties": {"passed": {"const": false}}}, "then": {"properties": {"reasons": {"minItems": 1}}}}]))]
pub struct Gate {
    /// Whether the document passed.
    pub passed: bool,
    /// Each failed threshold; empty when passed.
    #[serde(deserialize_with = "non_empty_reasons")]
    #[schemars(inner(length(min = 1)))]
    pub reasons: Vec<String>,
}

fn non_empty_reasons<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let reasons = Vec::<String>::deserialize(d)?;
    if reasons.iter().any(String::is_empty) {
        return Err(serde::de::Error::custom("gate reasons must not be empty"));
    }
    Ok(reasons)
}

/// The whole result for one linted file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct Document {
    /// File path.
    #[schemars(length(min = 1))]
    pub path: String,
    /// Profile name.
    #[schemars(length(min = 1))]
    pub profile: String,
    /// Genre.
    pub genre: GenreResult,
    /// Contexts found in the document.
    #[schemars(extend("uniqueItems" = true))]
    pub contexts_seen: Vec<ContextTag>,
    /// Config layers that applied, lowest priority first.
    pub config_sources: Vec<ConfigSource>,
    /// Words.
    pub words: usize,
    /// Sentences.
    pub sentences: usize,
    /// Paragraphs.
    pub paragraphs: usize,
    /// Findings.
    pub findings: Vec<Finding>,
    /// Per-category scores.
    pub categories: Vec<CategoryScore>,
    /// Total findings.
    pub total_findings: usize,
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Suggestions.
    pub suggestions: usize,
    /// Findings per 100 words.
    #[schemars(range(min = 0))]
    pub per_100_words: f64,
    /// Score, 0 to 100.
    #[schemars(range(min = 0, max = 100))]
    pub score: f64,
    /// Gate result.
    pub gate: Gate,
    /// What did not run, and why.
    pub unchecked: Vec<Notice>,
    /// AI register.
    pub ai_register: AiRegister,
    /// Prose axis.
    pub prose: AiRegisterSummary,
}

/// The `slopvac lint --format json` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct Report {
    /// Always 3.
    pub schema_version: SchemaVersion,
    /// Version of the tool that wrote the report.
    #[schemars(length(min = 1))]
    pub slopvac_version: String,
    /// Run summary.
    pub summary: RunSummary,
    /// Documents.
    pub documents: Vec<Document>,
    /// Run-level notices.
    pub notices: Vec<Notice>,
}
