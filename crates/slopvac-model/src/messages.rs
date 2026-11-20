//! Catalog text for model validation errors.

use slopvac_messages::{Catalog, Entry};

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
