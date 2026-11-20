//! Catalog text for model validation errors.

use slopvac_messages::{Catalog, Entry};

pub(crate) fn edit_position(reason: &crate::PositionError) -> ModelText {
    ModelText::EditPosition {
        reason: reason.to_string(),
    }
}

pub(crate) fn gate_inconsistent(passed: bool, count: usize) -> ModelText {
    ModelText::GateInconsistent {
        passed: passed.to_string(),
        count,
    }
}

pub(crate) fn origin_locator(reason: &str) -> ModelText {
    ModelText::OriginLocator {
        reason: reason.into(),
    }
}

pub(crate) fn origin_label(reason: &str) -> ModelText {
    ModelText::OriginLabel {
        reason: reason.into(),
    }
}

#[derive(Debug, Catalog)]
pub(crate) enum ModelText {
    /// An identifier is malformed.
    #[code("model-invalid-id")]
    #[text("invalid {kind} {value}")]
    InvalidId { kind: String, value: String },
    /// A scalar breaks its constraint.
    #[code("model-invalid-value")]
    #[text("invalid {kind}: {value}")]
    InvalidValue { kind: String, value: String },
    /// A tag appears more than once.
    #[code("model-duplicate-context")]
    #[text("context tag {tag} is listed twice")]
    DuplicateContext { tag: String },
    /// A schema version differs from the supported version.
    #[code("model-schema-version")]
    #[text("schema_version must be {expected}, got {actual}")]
    SchemaVersion { expected: String, actual: String },
    /// An origin locator is malformed.
    #[code("model-origin-locator")]
    #[text("invalid origin locator: {reason}")]
    OriginLocator { reason: String },
    /// An origin label is malformed.
    #[code("model-origin-label")]
    #[text("invalid origin label: {reason}")]
    OriginLabel { reason: String },
    /// A segment identifier is malformed.
    #[code("model-segment-id")]
    #[text("invalid segment id")]
    SegmentId,
    /// A segment range is reversed.
    #[code("model-segment-range")]
    #[text("segment range start {start} is after end {end}")]
    SegmentRange {
        #[count]
        start: usize,
        #[count]
        end: usize,
    },
    /// A source span is reversed.
    #[code("model-span-reversed")]
    #[text("span start {start} is after end {end}")]
    SpanReversed {
        #[count]
        start: usize,
        #[count]
        end: usize,
    },
    /// A position uses a zero line or column.
    #[code("model-line-column")]
    #[text("line and column are one-based, got {line}:{column}")]
    LineColumn {
        #[count]
        line: usize,
        #[count]
        column: usize,
    },
    /// An offset is beyond the source.
    #[code("model-offset-past-end")]
    #[text("offset {offset} is past the end of the file")]
    OffsetPastEnd {
        #[count]
        offset: usize,
    },
    /// An offset splits a byte order mark.
    #[code("model-offset-bom")]
    #[text("offset {offset} is inside the byte order mark")]
    OffsetBom {
        #[count]
        offset: usize,
    },
    /// An offset splits a code point.
    #[code("model-offset-code-point")]
    #[text("offset {offset} is inside a code point")]
    OffsetCodePoint {
        #[count]
        offset: usize,
    },
    /// An offset splits a CRLF pair.
    #[code("model-offset-crlf")]
    #[text("offset {offset} is between the CR and LF of one CRLF")]
    OffsetCrlf {
        #[count]
        offset: usize,
    },
    /// An edit carries an invalid position.
    #[code("model-edit-position")]
    #[text("invalid edit span: {reason}")]
    EditPosition { reason: String },
    /// Two edits overlap.
    #[code("model-edit-overlap")]
    #[text("edits at {first} and {second} overlap")]
    EditOverlap { first: String, second: String },
    /// A fix has no edits.
    #[code("model-fix-empty")]
    #[text("a fix needs at least one edit")]
    FixEmpty,
    /// A finding carries only part of its range.
    #[code("model-finding-range")]
    #[text("span, end_line and end_column must all be present or all absent")]
    FindingRange,
    /// An extracted finding carries a fix.
    #[code("model-finding-origin-fix")]
    #[text("a finding with an origin cannot carry a fix")]
    FindingOriginFix,
    /// A failed gate has no reasons.
    #[code("model-gate-empty")]
    #[text("a failed gate needs at least one reason")]
    GateEmpty,
    /// A gate reason is empty.
    #[code("model-gate-empty-reason")]
    #[text("a gate reason must not be empty")]
    GateEmptyReason,
    /// A gate status contradicts its reasons.
    #[code("model-gate-inconsistent")]
    #[text("passed is {passed} but there are {count} reasons")]
    GateInconsistent {
        passed: String,
        #[count]
        count: usize,
    },
}

/// Model validation text registered by the binary.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    ModelText::entries()
}

#[derive(Debug, Catalog)]
pub(crate) enum ModelPhrase {
    /// An empty value.
    #[code("model-reason-empty")]
    #[text("empty")]
    Empty,
    /// A value is too long.
    #[code("model-reason-too-long")]
    #[text("too long")]
    TooLong,
    /// A locator contains an unescaped forbidden character.
    #[code("model-reason-locator-character")]
    #[text("forbidden character, percent-escape it")]
    LocatorCharacter,
    /// A locator has an invalid percent escape.
    #[code("model-reason-locator-escape")]
    #[text("`%` must start %XX with uppercase hex")]
    LocatorEscape,
    /// A label contains a forbidden character.
    #[code("model-reason-label-character")]
    #[text("forbidden character")]
    LabelCharacter,
    /// A text scalar is empty.
    #[code("model-reason-empty-string")]
    #[text("empty string")]
    EmptyString,
    /// The plugin identifier label.
    #[code("model-kind-plugin")]
    #[text("plugin id")]
    PluginId,
    /// The rule identifier label.
    #[code("model-kind-rule")]
    #[text("rule id")]
    RuleId,
    /// The context tag label.
    #[code("model-kind-context")]
    #[text("context tag")]
    ContextTag,
    /// The API version label.
    #[code("model-kind-api-version")]
    #[text("api version")]
    ApiVersion,
    /// The category identifier label.
    #[code("model-kind-category")]
    #[text("category id")]
    CategoryId,
    /// The genre identifier label.
    #[code("model-kind-genre")]
    #[text("genre id")]
    GenreId,
    /// The kebab-case name label.
    #[code("model-kind-kebab")]
    #[text("kebab-case name")]
    KebabName,
    /// The density label.
    #[code("model-kind-density")]
    #[text("density")]
    Density,
    /// The score label.
    #[code("model-kind-score")]
    #[text("score")]
    Score,
    /// The text label.
    #[code("model-kind-text")]
    #[text("text")]
    Text,
    /// The schema version description.
    #[code("model-schema-version-description")]
    #[text("Root artifact schema version.")]
    SchemaVersionDescription,
    /// The density description.
    #[code("model-density-description")]
    #[text("Findings per 100 words.")]
    DensityDescription,
    /// The score description.
    #[code("model-score-description")]
    #[text("Quality score from 0 (worst) to 100.")]
    ScoreDescription,
    /// The genre description.
    #[code("model-genre-description")]
    #[text("Genre id.")]
    GenreDescription,
    /// The source-span description.
    #[code("model-span-description")]
    #[text(
        "Half-open range [start, end) of byte offsets into the original file bytes, BOM included. The end is exclusive, start is at most end, and both offsets fall on code point boundaries and never between the CR and LF of one CRLF. See docs/contract/positions.md."
    )]
    SpanDescription,
    /// The source-position description.
    #[code("model-position-description")]
    #[text(
        "One-based line and column. The column counts Unicode code points from the line start, after any BOM, and a tab counts as one. Output formats that need another unit convert from the span. See docs/contract/positions.md."
    )]
    PositionDescription,
}

/// Model fixed phrases registered by the binary.
#[must_use]
pub fn phrase_entries() -> &'static [Entry] {
    ModelPhrase::entries()
}

pub(crate) fn span_description(schema: &mut schemars::Schema) {
    schema.insert(
        "description".into(),
        ModelPhrase::SpanDescription.as_static().into(),
    );
}

pub(crate) fn position_description(schema: &mut schemars::Schema) {
    schema.insert(
        "description".into(),
        ModelPhrase::PositionDescription.as_static().into(),
    );
}

pub(crate) fn invalid_id(kind: &str, value: &str) -> String {
    ModelText::InvalidId {
        kind: kind.into(),
        value: format!("{value:?}"),
    }
    .to_string()
}

pub(crate) fn invalid_value(kind: &str, value: &str) -> String {
    ModelText::InvalidValue {
        kind: kind.into(),
        value: value.into(),
    }
    .to_string()
}

pub(crate) fn duplicate_context(tag: &str) -> String {
    ModelText::DuplicateContext {
        tag: format!("{tag:?}"),
    }
    .to_string()
}

pub(crate) fn overlap(first: &crate::Span, second: &crate::Span) -> String {
    ModelText::EditOverlap {
        first: format!("{first:?}"),
        second: format!("{second:?}"),
    }
    .to_string()
}
