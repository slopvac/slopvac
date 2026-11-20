use std::num::NonZeroUsize;

use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::edit::{Edit, EditError, sorted_without_overlap};
use crate::ids::{Contexts, PackId, RuleId, SchemaVersion, Severity};
use crate::origin::Origin;
use crate::schema::{absent_not_null, dependent_required, if_then, open_object, x_extensions};
use crate::span::{LineCol, LineColError, Span};
use crate::values::{
    CategoryId, Density, GenreId, KebabName, NonEmptyString, Score, present, required_nullable,
};

/// `budget` is always written, as a non-negative number or `null`.
fn nullable_budget(_: &mut schemars::SchemaGenerator) -> Schema {
    schemars::json_schema!({ "type": ["number", "null"], "minimum": 0 })
}

/// The schema of a count: a non-negative integer, shared as `$defs/Count`.
struct Count;

impl JsonSchema for Count {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Count".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> Schema {
        schemars::json_schema!({ "type": "integer", "minimum": 0 })
    }
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
#[schemars(inline)]
pub enum NoticeLevel {
    /// Informational.
    Info,
    /// Needs attention.
    Warning,
}

/// A stable, machine-readable fact with a human message. Consumers branch on `code`, never on
/// `message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct Notice {
    /// Stable kebab-case identifier. A code is never reused for a different meaning.
    pub code: KebabName,
    /// Level.
    pub level: NoticeLevel,
    /// Human message.
    pub message: NonEmptyString,
    /// File the notice concerns, when it concerns one.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    pub path: Option<String>,
    /// Details as a JSON object; a value may be any JSON value, including a nested object or
    /// array. Each notice code documents its own keys. Consumers must ignore keys they do
    /// not know. Omitted when the notice has none.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(transform = open_object)]
    pub data: serde_json::Map<String, Value>,
}

impl Notice {
    /// Render a notice from a compiled catalog. The catalog's code must be a kebab-case
    /// identifier, as checked by `#[derive(Catalog)]`. An empty rendering uses the entry's
    /// nonempty summary instead.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "codes and summaries are validated compiled catalog data"
    )]
    pub fn from_catalog(level: NoticeLevel, catalog: &impl slopvac_messages::Catalog) -> Self {
        let message = slopvac_messages::render(catalog);
        Self {
            code: KebabName::new(catalog.code()).expect("compiled catalog code is valid"),
            level,
            message: NonEmptyString::new(message).unwrap_or_else(|_| {
                NonEmptyString::new(catalog.entry().summary)
                    .expect("compiled catalog summary is nonempty")
            }),
            path: None,
            data: serde_json::Map::new(),
        }
    }

    /// Returns the notice with `data` attached.
    #[must_use]
    pub fn with_data(mut self, data: serde_json::Map<String, Value>) -> Self {
        self.data = data;
        self
    }
}

/// The pack that supplied a rule at run time: pack id, pack version and the pack-relative path
/// of the defining file. A rule's citation of its own sources is the rule field citation, not
/// this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
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
#[schemars(inline)]
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

/// Config layer and location a setting came from. Layer order and file semantics are in
/// docs/contract/config.md.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct ConfigSource {
    /// Layer name, lowest priority first: core, style, genre, extends, project, override,
    /// front-matter, directive, cli. env marks the sticky environment switches.
    pub layer: ConfigLayer,
    /// Path relative to the project root, or `plugin:<id>/<path>` for a plugin file. Absent where no
    /// file holds the value: core, cli and env.
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

/// A machine-applicable correction. Edits address source bytes and never overlap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "FixWire", into = "FixWire")]
#[schemars(with = "FixWire")]
pub struct Fix {
    edits: Vec<Edit>,
}

/// A machine-applicable correction. Edits address source bytes and never overlap.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Fix", transform = x_extensions)]
struct FixWire {
    /// Non-overlapping edits in original offsets.
    #[schemars(length(min = 1))]
    edits: Vec<Edit>,
}

/// Why a [`Fix`] cannot exist.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FixError {
    /// A fix needs at least one edit.
    #[error("{}", crate::messages::ModelText::FixEmpty)]
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
    #[error("{}", crate::messages::ModelText::FindingRange)]
    PartialExtent,
    /// A binary-host finding (one with an `origin`) cannot carry a `fix`.
    #[error("{}", crate::messages::ModelText::FindingOriginFix)]
    OriginWithFix,
}

/// One violation at one position. `line`, `column`, `end_line` and `end_column` are the display
/// position; `span` is the byte range. A finding with no source range, such as a metric over a
/// whole document, omits `span`, `end_line` and `end_column`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(try_from = "FindingWire")]
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
    /// Segment of a container document that holds the match, if any.
    pub origin: Option<Origin>,
}

impl Finding {
    /// Check the rules that the public fields cannot hold: a finding with an `origin`
    /// carries no `fix`. Serialization runs the same check.
    pub fn validate(&self) -> Result<(), FindingError> {
        if self.origin.is_some() && self.fix.is_some() {
            Err(FindingError::OriginWithFix)
        } else {
            Ok(())
        }
    }
}

impl Serialize for Finding {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        FindingWire::from(self.clone()).serialize(serializer)
    }
}

/// One violation at one position. `line`, `column`, `end_line` and `end_column` are the display
/// position; `span` is the byte range. A finding with no source range, such as a metric over a
/// whole document, omits `span`, `end_line` and `end_column`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(
    rename = "Finding",
    transform = x_extensions,
    extend(
        "dependentRequired" = dependent_required(&[
            ("span", &["end_line", "end_column"]),
            ("end_line", &["end_column", "span"]),
            ("end_column", &["end_line", "span"]),
        ]),
        "not" = {"required": ["origin", "fix"]}
    )
)]
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
    /// Text contexts the match sits in, such as prose or code-comment.
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
    /// Segment of a container document that holds the match.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    origin: Option<Origin>,
}

impl TryFrom<FindingWire> for Finding {
    type Error = FindingError;
    fn try_from(w: FindingWire) -> Result<Self, FindingError> {
        if w.origin.is_some() && w.fix.is_some() {
            return Err(FindingError::OriginWithFix);
        }
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
            origin: w.origin,
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
            origin: f.origin,
        }
    }
}
/// Counts for one reporting axis, independent of scoring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct AiRegisterSummary {
    /// Findings.
    #[schemars(with = "Count")]
    pub findings: usize,
    /// Errors.
    #[schemars(with = "Count")]
    pub errors: usize,
    /// Warnings.
    #[schemars(with = "Count")]
    pub warnings: usize,
    /// Findings per 100 words.
    pub per_100_words: Density,
}

/// Strong and weak AI-signal summaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct AiRegister {
    /// Strong signal.
    pub strong: AiRegisterSummary,
    /// Weak signal.
    pub weak: AiRegisterSummary,
}

/// One category rolled up across every linted document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct CategorySummary {
    /// Category id.
    pub category: CategoryId,
    /// Findings.
    #[schemars(with = "Count")]
    pub findings: usize,
    /// Errors.
    #[schemars(with = "Count")]
    pub errors: usize,
    /// Warnings.
    #[schemars(with = "Count")]
    pub warnings: usize,
    /// Suggestions.
    #[schemars(with = "Count")]
    pub suggestions: usize,
    /// Findings per 100 words.
    pub per_100_words: Density,
    /// Score, 0 to 100.
    pub score: Score,
}

/// Every document rolled into one result. Densities use total words, not an average of
/// per-document densities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct RunSummary {
    /// Documents linted.
    #[schemars(with = "Count")]
    pub documents: usize,
    /// Total words.
    #[schemars(with = "Count")]
    pub words: usize,
    /// Findings.
    #[schemars(with = "Count")]
    pub findings: usize,
    /// Errors.
    #[schemars(with = "Count")]
    pub errors: usize,
    /// Warnings.
    #[schemars(with = "Count")]
    pub warnings: usize,
    /// Suggestions.
    #[schemars(with = "Count")]
    pub suggestions: usize,
    /// Findings per 100 words.
    pub per_100_words: Density,
    /// Score, 0 to 100.
    pub score: Score,
    /// True when every document passed its gate.
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
#[schemars(inline)]
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
#[schemars(transform = x_extensions)]
pub struct GenreReason {
    /// Kebab-case signal name, such as `front-matter` or `path-glob`.
    pub signal: KebabName,
    /// Detail.
    pub detail: String,
}

/// The genre applied to a document and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct GenreResult {
    /// Genre id.
    pub id: GenreId,
    /// `explicit` came from config, the command line or front matter; `detected` from genre
    /// signals; `default` from the fallback.
    pub source: GenreSource,
    /// Reasons.
    pub reasons: Vec<GenreReason>,
}

/// Per-category result for one document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct CategoryScore {
    /// Category id.
    pub category: CategoryId,
    /// Findings.
    #[schemars(with = "Count")]
    pub findings: usize,
    /// Errors.
    #[schemars(with = "Count")]
    pub errors: usize,
    /// Warnings.
    #[schemars(with = "Count")]
    pub warnings: usize,
    /// Suggestions.
    #[schemars(with = "Count")]
    pub suggestions: usize,
    /// Findings per 100 words.
    pub per_100_words: Density,
    /// Errors and warnings per 100 words: the figure the budget is checked against.
    pub gating_per_100_words: Density,
    /// Gating density limit, or null when the category has none.
    #[serde(deserialize_with = "required_nullable")]
    #[schemars(schema_with = "nullable_budget")]
    pub budget: Option<Density>,
    /// Score, 0 to 100.
    pub score: Score,
    /// Whether the budget is exceeded.
    pub over_budget: bool,
}

/// The pass or fail result for one document under its profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "GateWire", into = "GateWire")]
#[schemars(with = "GateWire")]
pub struct Gate {
    reasons: Vec<String>,
}

/// The pass or fail result for one document under its profile.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(
    rename = "Gate",
    transform = x_extensions,
    extend("allOf" = [
        if_then(
            &json!({"properties": {"passed": {"const": true}}}),
            &json!({"properties": {"reasons": {"maxItems": 0}}}),
        ),
        if_then(
            &json!({"properties": {"passed": {"const": false}}}),
            &json!({"properties": {"reasons": {"minItems": 1}}}),
        ),
    ])
)]
struct GateWire {
    /// Whether the document passed.
    passed: bool,
    /// Each failed threshold, named. Empty when passed.
    #[schemars(inner(length(min = 1)))]
    reasons: Vec<String>,
}

/// Why a [`Gate`] cannot exist.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GateError {
    /// A failed gate names at least one reason.
    #[error("{}", crate::messages::ModelText::GateEmpty)]
    NoReasons,
    /// A reason is an empty string.
    #[error("{}", crate::messages::ModelText::GateEmptyReason)]
    EmptyReason,
    /// `passed` disagrees with the reasons.
    #[error("{}", crate::messages::gate_inconsistent(*.passed, *.count))]
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
#[schemars(transform = x_extensions)]
pub struct Document {
    /// File path.
    pub path: NonEmptyString,
    /// Profile name.
    pub profile: NonEmptyString,
    /// Genre.
    pub genre: GenreResult,
    /// Every text context found in the document, whether or not a finding fired in it.
    pub contexts_seen: Contexts,
    /// Config layers that applied to this document, lowest priority first.
    pub config_sources: Vec<ConfigSource>,
    /// Words.
    #[schemars(with = "Count")]
    pub words: usize,
    /// Sentences.
    #[schemars(with = "Count")]
    pub sentences: usize,
    /// Paragraphs.
    #[schemars(with = "Count")]
    pub paragraphs: usize,
    /// Findings.
    pub findings: Vec<Finding>,
    /// Per-category scores.
    pub categories: Vec<CategoryScore>,
    /// Total findings.
    #[schemars(with = "Count")]
    pub total_findings: usize,
    /// Errors.
    #[schemars(with = "Count")]
    pub errors: usize,
    /// Warnings.
    #[schemars(with = "Count")]
    pub warnings: usize,
    /// Suggestions.
    #[schemars(with = "Count")]
    pub suggestions: usize,
    /// Findings per 100 words.
    pub per_100_words: Density,
    /// Score, 0 to 100.
    pub score: Score,
    /// Gate result.
    pub gate: Gate,
    /// What did not run, and why. A skipped check must never read as a pass.
    pub unchecked: Vec<Notice>,
    /// AI register.
    pub ai_register: AiRegister,
    /// Prose axis.
    pub prose: AiRegisterSummary,
}

/// The `slopvac lint --format json` payload. Spec: docs/contract/lint-report.md. Keys starting
/// with `x-` are ignored by consumers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct Report {
    /// Always 3.
    pub schema_version: SchemaVersion,
    /// Version of the tool that wrote the report. Not a schema version.
    pub slopvac_version: NonEmptyString,
    /// Run summary.
    pub summary: RunSummary,
    /// Documents.
    pub documents: Vec<Document>,
    /// Run-level facts a reader needs, such as skipped files or checks that did not run. Always
    /// present; empty when there is nothing to say.
    pub notices: Vec<Notice>,
}
