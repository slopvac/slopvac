use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Half-open range `[start, end)` of byte offsets into the original file bytes.
///
/// Construction does not validate against a file. Use [`crate::SourceIndex::check_span`]
/// for that.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct Span {
    /// First byte of the span.
    pub start: usize,
    /// First byte after the span.
    pub end: usize,
}

impl Span {
    /// Build a span from two offsets.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// A zero-width span at `offset`.
    #[must_use]
    pub const fn empty(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// Whether the span has no width.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Length in bytes, or 0 when `end` precedes `start`.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

/// One-based line and column. The column counts code points unless a
/// [`crate::ColumnUnit`] says otherwise.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct LineCol {
    /// One-based line.
    pub line: usize,
    /// One-based column.
    pub column: usize,
}

/// Why an offset or span is not valid for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PositionError {
    /// The offset is past the end of the file.
    #[error("offset {0} is past the end of the file")]
    PastEnd(usize),
    /// The offset is inside the byte order mark.
    #[error("offset {0} is inside the byte order mark")]
    InsideBom(usize),
    /// The offset is not on a code point boundary.
    #[error("offset {0} is inside a code point")]
    InsideCodePoint(usize),
    /// The offset is between the CR and LF of one CRLF.
    #[error("offset {0} is between the CR and LF of one CRLF")]
    InsideCrlf(usize),
    /// The span starts after it ends.
    #[error("span start {start} is after end {end}")]
    Reversed {
        /// Start offset.
        start: usize,
        /// End offset.
        end: usize,
    },
}
