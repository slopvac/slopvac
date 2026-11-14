use std::num::NonZeroUsize;

use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::edit::{Edit, EditError, sorted_without_overlap};
use crate::ids::{Contexts, PackId, RuleId, SchemaVersion, Severity};
use crate::span::{LineCol, LineColError, Span};
use crate::values::{
    CategoryId, Density, GenreId, KebabName, NonEmptyString, Score, present, required_nullable,
};

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
    pub code: KebabName,
    /// Level.
    pub level: NoticeLevel,
    /// Human message.
    pub message: NonEmptyString,
    /// File the notice concerns.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    pub version: Option<String>,
    /// Pack source.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    pub source: Option<String>,
}

/// A config layer, lowest priority first; `env` marks the sticky environment switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigLayer {
    /// Built-in defaults.
    Core,
    /// Style pack.
    Style,
    /// Genre settings.
    Genre,
    /// Extended config.
    Extends,
    /// Project config file.
    Project,
    /// Override block.
    Override,
    /// Document front matter.
    FrontMatter,
    /// Inline directive.
    Directive,
    /// Command line.
    Cli,
    /// Environment switches.
    Env,
}

/// Config layer and location a setting came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct ConfigSource {
    /// Layer name.
    pub layer: ConfigLayer,
    /// File.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    pub file: Option<String>,
    /// One-based line.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    pub line: Option<NonZeroUsize>,
    /// One-based column.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    pub column: Option<NonZeroUsize>,
}

/// A machine-applicable correction: at least one edit, none overlapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "FixWire", into = "FixWire")]
#[schemars(with = "FixWire")]
pub struct Fix {
    edits: Vec<Edit>,
}

/// Wire shape of [`Fix`].
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Fix", extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
struct FixWire {
    /// Non-overlapping edits in original offsets.
    #[schemars(length(min = 1))]
    edits: Vec<Edit>,
}

/// Why a [`Fix`] cannot exist.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FixError {
    /// A fix needs at least one edit.
    #[error("a fix needs at least one edit")]
    Empty,
    /// Two edits overlap.
    #[error(transparent)]
    Overlap(#[from] EditError),
}

impl Fix {
    /// Build a fix, rejecting an empty or overlapping edit set.
    pub fn new(edits: Vec<Edit>) -> Result<Self, FixError> {
        if edits.is_empty() {
            return Err(FixError::Empty);
        }
        sorted_without_overlap(&edits)?;
        Ok(Self { edits })
    }

    /// The edits in array order.
    #[must_use]
    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }
}

impl TryFrom<FixWire> for Fix {
    type Error = FixError;
    fn try_from(wire: FixWire) -> Result<Self, FixError> {
        Self::new(wire.edits)
    }
}

impl From<Fix> for FixWire {
    fn from(fix: Fix) -> Self {
        Self { edits: fix.edits }
    }
}

/// Where a finding ends: the byte range and the display end position, always together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    /// Byte range of the match.
    pub span: Span,
    /// Display position of the end.
    pub end: LineCol,
}

/// Why a finding cannot exist.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FindingError {
    /// A line or column is zero.
    #[error(transparent)]
    Position(#[from] LineColError),
    /// `span`, `end_line` and `end_column` must all be present or all absent.
    #[error("span, end_line and end_column must all be present or all absent")]
    PartialExtent,
}

/// One violation at one position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "FindingWire", into = "FindingWire")]
#[schemars(with = "FindingWire")]
pub struct Finding {
    /// Qualified rule id.
    pub rule_id: RuleId,
    /// Owning pack.
    pub pack: PackId,
    /// Category id.
    pub category: CategoryId,
    /// Severity.
    pub severity: Severity,
    /// Message.
    pub message: NonEmptyString,
    /// Display position of the start.
    pub start: LineCol,
    /// Byte range and end position; absent for whole-document findings.
    pub extent: Option<Extent>,
    /// Text contexts the match sits in.
    pub contexts: Contexts,
    /// AI signal.
    pub ai_signal: AiSignal,
    /// Matched source text.
    pub matched_text: String,
    /// Suggested fix.
    pub fix: Option<Fix>,
    /// Rule provenance.
    pub provenance: Option<RuleProvenance>,
}

/// Wire shape of [`Finding`].
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Finding", extend("additionalProperties" = false, "patternProperties" = {"^x-": true}, "dependentRequired" = {"span": ["end_line", "end_column"], "end_line": ["end_column", "span"], "end_column": ["end_line", "span"]}))]
struct FindingWire {
    /// Qualified rule id.
    rule_id: RuleId,
    /// Owning pack.
    pack: PackId,
    /// Category id.
    category: CategoryId,
    /// Severity.
    severity: Severity,
    /// Message.
    message: NonEmptyString,
    /// Byte range; absent for whole-document findings.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    span: Option<Span>,
    /// Start line.
    #[schemars(range(min = 1))]
    line: usize,
    /// Start column.
    #[schemars(range(min = 1))]
    column: usize,
    /// End line; present with `span`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(range(min = 1), transform = absent_not_null)]
    end_line: Option<usize>,
    /// End column; present with `span`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(range(min = 1), transform = absent_not_null)]
    end_column: Option<usize>,
    /// Text contexts the match sits in.
    contexts: Contexts,
    /// AI signal.
    ai_signal: AiSignal,
    /// Matched source text.
    matched_text: String,
    /// Suggested fix.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    fix: Option<Fix>,
    /// Rule provenance.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    provenance: Option<RuleProvenance>,
}

impl TryFrom<FindingWire> for Finding {
    type Error = FindingError;
    fn try_from(w: FindingWire) -> Result<Self, FindingError> {
        let extent = match (w.span, w.end_line, w.end_column) {
            (None, None, None) => None,
            (Some(span), Some(line), Some(column)) => Some(Extent {
                span,
                end: LineCol::new(line, column)?,
            }),
            _ => return Err(FindingError::PartialExtent),
        };
        Ok(Self {
            rule_id: w.rule_id,
            pack: w.pack,
            category: w.category,
            severity: w.severity,
            message: w.message,
            start: LineCol::new(w.line, w.column)?,
            extent,
            contexts: w.contexts,
            ai_signal: w.ai_signal,
            matched_text: w.matched_text,
            fix: w.fix,
            provenance: w.provenance,
        })
    }
}

impl From<Finding> for FindingWire {
    fn from(f: Finding) -> Self {
        Self {
            rule_id: f.rule_id,
            pack: f.pack,
            category: f.category,
            severity: f.severity,
            message: f.message,
            span: f.extent.map(|e| e.span),
            line: f.start.line(),
            column: f.start.column(),
            end_line: f.extent.map(|e| e.end.line()),
            end_column: f.extent.map(|e| e.end.column()),
            contexts: f.contexts,
            ai_signal: f.ai_signal,
            matched_text: f.matched_text,
            fix: f.fix,
            provenance: f.provenance,
        }
    }
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
    pub per_100_words: Density,
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
    pub category: CategoryId,
    /// Findings.
    pub findings: usize,
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Suggestions.
    pub suggestions: usize,
    /// Findings per 100 words.
    pub per_100_words: Density,
    /// Score, 0 to 100.
    pub score: Score,
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
    pub per_100_words: Density,
    /// Score, 0 to 100.
    pub score: Score,
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
    pub signal: KebabName,
    /// Detail.
    pub detail: String,
}

/// The genre applied to a document and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct GenreResult {
    /// Genre id.
    pub id: GenreId,
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
    pub category: CategoryId,
    /// Findings.
    pub findings: usize,
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Suggestions.
    pub suggestions: usize,
    /// Findings per 100 words.
    pub per_100_words: Density,
    /// Errors and warnings per 100 words.
    pub gating_per_100_words: Density,
    /// Gating limit, or null.
    #[serde(deserialize_with = "required_nullable")]
    #[schemars(schema_with = "nullable_budget")]
    pub budget: Option<Density>,
    /// Score, 0 to 100.
    pub score: Score,
    /// Whether the budget is exceeded.
    pub over_budget: bool,
}

/// Pass or fail result for one document. It passes exactly when no reason is recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "GateWire", into = "GateWire")]
#[schemars(with = "GateWire")]
pub struct Gate {
    reasons: Vec<String>,
}

/// Wire shape of [`Gate`].
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Gate", extend("additionalProperties" = false, "patternProperties" = {"^x-": true}, "allOf" = [{"if": {"properties": {"passed": {"const": true}}}, "then": {"properties": {"reasons": {"maxItems": 0}}}}, {"if": {"properties": {"passed": {"const": false}}}, "then": {"properties": {"reasons": {"minItems": 1}}}}]))]
struct GateWire {
    /// Whether the document passed.
    passed: bool,
    /// Each failed threshold; empty when passed.
    #[schemars(inner(length(min = 1)))]
    reasons: Vec<String>,
}

/// Why a [`Gate`] cannot exist.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GateError {
    /// A failed gate names at least one reason.
    #[error("a failed gate needs at least one reason")]
    NoReasons,
    /// A reason is an empty string.
    #[error("a gate reason must not be empty")]
    EmptyReason,
    /// `passed` disagrees with the reasons.
    #[error("passed is {passed} but there are {count} reasons")]
    Mismatch {
        /// The `passed` value given.
        passed: bool,
        /// How many reasons came with it.
        count: usize,
    },
}

impl Gate {
    /// A gate that passed.
    #[must_use]
    pub const fn passed() -> Self {
        Self {
            reasons: Vec::new(),
        }
    }

    /// A gate that failed, naming each failed threshold.
    pub fn failed(reasons: Vec<String>) -> Result<Self, GateError> {
        if reasons.is_empty() {
            return Err(GateError::NoReasons);
        }
        if reasons.iter().any(String::is_empty) {
            return Err(GateError::EmptyReason);
        }
        Ok(Self { reasons })
    }

    /// Whether the document passed.
    #[must_use]
    pub fn is_passed(&self) -> bool {
        self.reasons.is_empty()
    }

    /// The failed thresholds; empty when passed.
    #[must_use]
    pub fn reasons(&self) -> &[String] {
        &self.reasons
    }
}

impl TryFrom<GateWire> for Gate {
    type Error = GateError;
    fn try_from(wire: GateWire) -> Result<Self, GateError> {
        if wire.passed != wire.reasons.is_empty() {
            return Err(GateError::Mismatch {
                passed: wire.passed,
                count: wire.reasons.len(),
            });
        }
        if wire.passed {
            Ok(Self::passed())
        } else {
            Self::failed(wire.reasons)
        }
    }
}

impl From<Gate> for GateWire {
    fn from(gate: Gate) -> Self {
        Self {
            passed: gate.reasons.is_empty(),
            reasons: gate.reasons,
        }
    }
}
/// The whole result for one linted file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
pub struct Document {
    /// File path.
    pub path: NonEmptyString,
    /// Profile name.
    pub profile: NonEmptyString,
    /// Genre.
    pub genre: GenreResult,
    /// Contexts found in the document.
    pub contexts_seen: Contexts,
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
    pub per_100_words: Density,
    /// Score, 0 to 100.
    pub score: Score,
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
    pub slopvac_version: NonEmptyString,
    /// Run summary.
    pub summary: RunSummary,
    /// Documents.
    pub documents: Vec<Document>,
    /// Run-level notices.
    pub notices: Vec<Notice>,
}
