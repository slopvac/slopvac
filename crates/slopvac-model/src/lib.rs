//! Core types shared by every slopvac crate.
//!
//! The shapes match `schemas/v3`. Positions follow `docs/contract/positions.md`: a
//! [`Span`] of byte offsets is the only stored position, and [`SourceIndex`] derives
//! line and column from it.

mod edit;
mod ids;
mod origin;
mod report;
mod source;
mod span;
mod values;

pub use edit::{Edit, EditError, apply_edits};
pub use ids::{
    ApiVersion, ContextTag, Contexts, DuplicateContext, IdError, PackId, RuleId, SchemaVersion,
    Severity,
};
pub use origin::{Origin, OriginError, OriginLabel, OriginLocator, SegmentId, SegmentRange};
pub use report::{
    AiRegister, AiRegisterSummary, AiSignal, CategoryScore, CategorySummary, ConfigLayer,
    ConfigSource, Document, Extent, Finding, FindingError, Fix, FixError, Gate, GateError,
    GenreReason, GenreResult, GenreSource, Notice, NoticeLevel, Report, RuleProvenance, RunSummary,
};
#[cfg(feature = "test-counters")]
#[doc(hidden)]
pub use source::indexes_built;
pub use source::{BomMode, ColumnUnit, PositionEncoding, SourceIndex};
pub use span::{LineCol, LineColError, PositionError, Span, SpanError};
pub use values::{CategoryId, Density, GenreId, KebabName, NonEmptyString, Score, ValueError};
