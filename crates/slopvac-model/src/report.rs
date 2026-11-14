use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::edit::Edit;
use crate::ids::{PackId, RuleId, Severity};
use crate::span::Span;

/// Value of `schema_version` in every v3 report.
pub const SCHEMA_VERSION: u32 = 3;

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
pub struct Notice {
    /// Stable kebab-case identifier.
    pub code: String,
    /// Level.
    pub level: NoticeLevel,
    /// Human message.
    pub message: String,
    /// File the notice concerns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Where a rule came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuleProvenance {
    /// Owning pack.
    pub pack: PackId,
    /// Pack version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Pack source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Config layer and location a setting came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSource {
    /// Layer name.
    pub layer: String,
    /// File.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// One-based line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// One-based column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}

/// A machine-applicable correction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Fix {
    /// Non-overlapping edits in original offsets.
    pub edits: Vec<Edit>,
}

/// One violation at one position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Finding {
    /// Qualified rule id.
    pub rule_id: RuleId,
    /// Owning pack.
    pub pack: PackId,
    /// Category id.
    pub category: String,
    /// Severity.
    pub severity: Severity,
    /// Message.
    pub message: String,
    /// Byte range; absent for whole-document findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    /// Start line.
    pub line: usize,
    /// Start column.
    pub column: usize,
    /// End line; present with `span`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    /// End column; present with `span`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_column: Option<usize>,
    /// Text contexts the match sits in.
    pub contexts: Vec<String>,
    /// AI signal.
    pub ai_signal: AiSignal,
    /// Matched source text.
    pub matched_text: String,
    /// Suggested fix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
    /// Rule provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<RuleProvenance>,
}

/// Counts for one reporting axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AiRegisterSummary {
    /// Findings.
    pub findings: usize,
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Findings per 100 words.
    pub per_100_words: f64,
}

/// Strong and weak AI-signal summaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AiRegister {
    /// Strong signal.
    pub strong: AiRegisterSummary,
    /// Weak signal.
    pub weak: AiRegisterSummary,
}

/// One category rolled up across every document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CategorySummary {
    /// Category id.
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
    pub per_100_words: f64,
    /// Score, 0 to 100.
    pub score: f64,
}

/// Every document rolled into one result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    pub per_100_words: f64,
    /// Score, 0 to 100.
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
pub struct GenreReason {
    /// Kebab-case signal name.
    pub signal: String,
    /// Detail.
    pub detail: String,
}

/// The genre applied to a document and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GenreResult {
    /// Genre id.
    pub id: String,
    /// Source of the choice.
    pub source: GenreSource,
    /// Reasons.
    pub reasons: Vec<GenreReason>,
}

/// Per-category result for one document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CategoryScore {
    /// Category id.
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
    pub per_100_words: f64,
    /// Errors and warnings per 100 words.
    pub gating_per_100_words: f64,
    /// Gating limit, or null.
    pub budget: Option<f64>,
    /// Score, 0 to 100.
    pub score: f64,
    /// Whether the budget is exceeded.
    pub over_budget: bool,
}

/// Pass or fail result for one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Gate {
    /// Whether the document passed.
    pub passed: bool,
    /// Each failed threshold; empty when passed.
    pub reasons: Vec<String>,
}

/// The whole result for one linted file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Document {
    /// File path.
    pub path: String,
    /// Profile name.
    pub profile: String,
    /// Genre.
    pub genre: GenreResult,
    /// Contexts found in the document.
    pub contexts_seen: Vec<String>,
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
    pub per_100_words: f64,
    /// Score, 0 to 100.
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
pub struct Report {
    /// Always [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Version of the tool that wrote the report.
    pub slopvac_version: String,
    /// Run summary.
    pub summary: RunSummary,
    /// Documents.
    pub documents: Vec<Document>,
    /// Run-level notices.
    pub notices: Vec<Notice>,
}
