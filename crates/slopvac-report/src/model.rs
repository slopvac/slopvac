//! The v3 lint report (docs/contract/lint-report.md, schemas/v3/lint-report.schema.json).
//!
//! Every output format renders from this one model. Serialising it as JSON
//! gives `--format json`.

use serde::{Deserialize, Serialize};

/// Run-level count.
pub type Count = u64;

/// Finding severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Fails the gate.
    Error,
    /// Counts toward the gate.
    Warning,
    /// Informational.
    Suggestion,
}

/// Whether a finding is a sign of AI authorship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiSignal {
    /// Strong sign.
    Strong,
    /// Weak sign.
    Weak,
    /// Not a sign.
    None,
}

/// Notice level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    /// Information.
    Info,
    /// Something a reader should act on.
    Warning,
}

/// Where a genre came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenreSource {
    /// Config, command line or front matter.
    Explicit,
    /// Genre signals.
    Detected,
    /// Fallback.
    Default,
}

/// The schema version, always 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaVersion;

impl Serialize for SchemaVersion {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(3)
    }
}

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = u64::deserialize(d)?;
        if v == 3 {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("schema_version must be 3"))
        }
    }
}

/// A byte range in the original file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

/// One text replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    /// Original-file bytes to replace.
    pub span: Span,
    /// Replacement source text.
    pub text: String,
}

/// A fix: edits that apply together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fix {
    /// At least one edit.
    pub edits: Vec<Edit>,
}

/// Which pack, version and source supplied a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleProvenance {
    /// Pack ID.
    pub pack: String,
    /// Pack version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Where the pack came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// A stable-coded fact about the run or a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// Kebab-case code. Consumers branch on it.
    pub code: String,
    /// Level.
    pub level: NoticeLevel,
    /// Human message.
    pub message: String,
    /// File the notice concerns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// A rule match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// `<pack>:<id>`.
    pub rule_id: String,
    /// Pack ID.
    pub pack: String,
    /// Category ID.
    pub category: String,
    /// Severity.
    pub severity: Severity,
    /// Fix guidance.
    pub message: String,
    /// Byte range; absent for document-wide findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    /// 1-based start line.
    pub line: u32,
    /// 1-based start column in code points.
    pub column: u32,
    /// End line, present with `span`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    /// Exclusive end column, present with `span`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u32>,
    /// Text contexts of the match.
    pub contexts: Vec<String>,
    /// AI signal.
    pub ai_signal: AiSignal,
    /// The matched text.
    pub matched_text: String,
    /// Proposed fix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
    /// Rule provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<RuleProvenance>,
}

/// Counts for one AI-register bucket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiRegisterSummary {
    /// Findings.
    pub findings: Count,
    /// Errors.
    pub errors: Count,
    /// Warnings.
    pub warnings: Count,
    /// Density.
    pub per_100_words: f64,
}

/// Strong and weak AI-signal buckets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiRegister {
    /// Strong signals.
    pub strong: AiRegisterSummary,
    /// Weak signals.
    pub weak: AiRegisterSummary,
}

/// Per-category counts in the run summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategorySummary {
    /// Category ID.
    pub category: String,
    /// Findings.
    pub findings: Count,
    /// Errors.
    pub errors: Count,
    /// Warnings.
    pub warnings: Count,
    /// Suggestions.
    pub suggestions: Count,
    /// Density.
    pub per_100_words: f64,
    /// Score from 0 to 100.
    pub score: f64,
}

/// Totals for the whole run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    /// Documents checked.
    pub documents: Count,
    /// Words.
    pub words: Count,
    /// Findings.
    pub findings: Count,
    /// Errors.
    pub errors: Count,
    /// Warnings.
    pub warnings: Count,
    /// Suggestions.
    pub suggestions: Count,
    /// Density.
    pub per_100_words: f64,
    /// Score from 0 to 100.
    pub score: f64,
    /// True when every document passed.
    pub passed: bool,
    /// Per-category summaries.
    pub categories: Vec<CategorySummary>,
    /// AI register.
    pub ai_register: AiRegister,
    /// Prose-only counts.
    pub prose: AiRegisterSummary,
}

/// One reason a genre was chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenreReason {
    /// Kebab-case signal name.
    pub signal: String,
    /// Detail.
    pub detail: String,
}

/// The genre of a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenreResult {
    /// Genre ID.
    pub id: String,
    /// Where it came from.
    pub source: GenreSource,
    /// Why.
    pub reasons: Vec<GenreReason>,
}

/// A config layer that applied to a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSource {
    /// Layer name.
    pub layer: String,
    /// File, when the layer is a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// 1-based line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 1-based column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

/// Per-category score for one document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategoryScore {
    /// Category ID.
    pub category: String,
    /// Findings.
    pub findings: Count,
    /// Errors.
    pub errors: Count,
    /// Warnings.
    pub warnings: Count,
    /// Suggestions.
    pub suggestions: Count,
    /// Density.
    pub per_100_words: f64,
    /// Errors and warnings per 100 words.
    pub gating_per_100_words: f64,
    /// Limit, or null.
    pub budget: Option<f64>,
    /// Score from 0 to 100.
    pub score: f64,
    /// Whether the budget is exceeded.
    pub over_budget: bool,
}

/// The gate verdict for one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    /// Whether the document passed.
    pub passed: bool,
    /// Failed thresholds; empty when passed.
    pub reasons: Vec<String>,
}

/// One linted document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// Path as shown to the user; `<stdin>` for standard input.
    pub path: String,
    /// Profile name.
    pub profile: String,
    /// Genre.
    pub genre: GenreResult,
    /// Contexts found in the document.
    pub contexts_seen: Vec<String>,
    /// Config layers, lowest priority first.
    pub config_sources: Vec<ConfigSource>,
    /// Words.
    pub words: Count,
    /// Sentences.
    pub sentences: Count,
    /// Paragraphs.
    pub paragraphs: Count,
    /// Findings.
    pub findings: Vec<Finding>,
    /// Category scores.
    pub categories: Vec<CategoryScore>,
    /// Total findings.
    pub total_findings: Count,
    /// Errors.
    pub errors: Count,
    /// Warnings.
    pub warnings: Count,
    /// Suggestions.
    pub suggestions: Count,
    /// Density.
    pub per_100_words: f64,
    /// Score from 0 to 100.
    pub score: f64,
    /// Gate verdict.
    pub gate: Gate,
    /// What did not run.
    pub unchecked: Vec<Notice>,
    /// AI register.
    pub ai_register: AiRegister,
    /// Prose-only counts.
    pub prose: AiRegisterSummary,
}

/// The whole report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Always 3.
    pub schema_version: SchemaVersion,
    /// Tool version.
    pub slopvac_version: String,
    /// Run totals.
    pub summary: RunSummary,
    /// Documents.
    pub documents: Vec<Document>,
    /// Run-level notices.
    pub notices: Vec<Notice>,
}

impl Report {
    /// Render as pretty JSON with a trailing newline.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = serde_json::to_string_pretty(self).expect("report serialises");
        out.push('\n');
        out
    }
}
