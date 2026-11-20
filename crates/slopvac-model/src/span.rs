use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::schema::x_extensions;

/// Half-open range `[start, end)` of byte offsets into the original file bytes.
///
/// A span is never reversed: every constructor and `Deserialize` reject `start > end`.
/// Checks that need the file, such as code point boundaries, live in
/// [`crate::SourceIndex::check_span`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "SpanRepr")]
#[schemars(
    with = "SpanRepr",
    description = "Half-open range [start, end) of byte offsets into the original file bytes, \
        BOM included. The end is exclusive, start is at most end, and both offsets fall on code \
        point boundaries and never between the CR and LF of one CRLF. See \
        docs/contract/positions.md."
)]
pub struct Span {
    start: usize,
    end: usize,
}

/// Wire shape of [`Span`].
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Span", transform = x_extensions)]
struct SpanRepr {
    start: usize,
    end: usize,
}

/// A span whose start is after its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{}", crate::messages::ModelText::SpanReversed { start: *.start, end: *.end })]
pub struct SpanError {
    /// Start offset.
    pub start: usize,
    /// End offset.
    pub end: usize,
}

impl TryFrom<SpanRepr> for Span {
    type Error = SpanError;
    fn try_from(repr: SpanRepr) -> Result<Self, SpanError> {
        Self::new(repr.start, repr.end)
    }
}

impl Span {
    /// Build a span, rejecting `start > end`.
    pub const fn new(start: usize, end: usize) -> Result<Self, SpanError> {
        if start > end {
            Err(SpanError { start, end })
        } else {
            Ok(Self { start, end })
        }
    }

    /// A zero-width span at `offset`.
    #[must_use]
    pub const fn empty(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// First byte of the span.
    #[must_use]
    pub const fn start(&self) -> usize {
        self.start
    }

    /// First byte after the span.
    #[must_use]
    pub const fn end(&self) -> usize {
        self.end
    }

    /// Whether the span has no width.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Length in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.end - self.start
    }
}

/// One-based line and column. The column counts code points unless a
/// [`crate::ColumnUnit`] says otherwise. Neither number is ever zero.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "LineColRepr")]
#[schemars(
    with = "LineColRepr",
    description = "One-based line and column. The column counts Unicode code points from the \
        line start, after any BOM, and a tab counts as one. Output formats that need another \
        unit convert from the span. See docs/contract/positions.md."
)]
pub struct LineCol {
    line: usize,
    column: usize,
}

/// Wire shape of [`LineCol`].
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "LineCol", extend("additionalProperties" = false))]
struct LineColRepr {
    #[schemars(range(min = 1))]
    line: usize,
    #[schemars(range(min = 1))]
    column: usize,
}

/// A line or column of zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{}", crate::messages::ModelText::LineColumn { line: *.line, column: *.column })]
pub struct LineColError {
    /// Line given.
    pub line: usize,
    /// Column given.
    pub column: usize,
}

impl TryFrom<LineColRepr> for LineCol {
    type Error = LineColError;
    fn try_from(repr: LineColRepr) -> Result<Self, LineColError> {
        Self::new(repr.line, repr.column)
    }
}

impl LineCol {
    /// Build a position, rejecting a zero line or column.
    pub const fn new(line: usize, column: usize) -> Result<Self, LineColError> {
        if line == 0 || column == 0 {
            Err(LineColError { line, column })
        } else {
            Ok(Self { line, column })
        }
    }

    /// One-based line.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// One-based column.
    #[must_use]
    pub const fn column(&self) -> usize {
        self.column
    }
}

/// Why an offset or span is not valid for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PositionError {
    /// The offset is past the end of the file.
    #[error("{}", crate::messages::ModelText::OffsetPastEnd { offset: *.0 })]
    PastEnd(usize),
    /// The offset is inside the byte order mark.
    #[error("{}", crate::messages::ModelText::OffsetBom { offset: *.0 })]
    InsideBom(usize),
    /// The offset is not on a code point boundary.
    #[error("{}", crate::messages::ModelText::OffsetCodePoint { offset: *.0 })]
    InsideCodePoint(usize),
    /// The offset is between the CR and LF of one CRLF.
    #[error("{}", crate::messages::ModelText::OffsetCrlf { offset: *.0 })]
    InsideCrlf(usize),
}
