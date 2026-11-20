//! Core types shared by every slopvac crate.
//!
//! The shapes match `schemas/v3`. Positions follow `docs/contract/positions.md`: a
//! [`Span`] of byte offsets is the only stored position, and [`SourceIndex`] derives
//! line and column from it.

mod edit;
mod ids;
mod messages;
mod origin;
mod report;
mod schema;
mod source;
mod span;
mod values;

pub use edit::{Edit, EditError, apply_edits};
pub use ids::{
    ApiVersion, ContextTag, Contexts, DuplicateContext, IdError, PackId, RuleId, SchemaVersion,
    Severity,
};
pub use messages::message_entries;
pub use origin::{
    Origin, OriginError, OriginLabel, OriginLocator, Result as OriginResult, SegmentId,
    SegmentRange,
};
pub use report::{
    AiRegister, AiRegisterSummary, AiSignal, CategoryScore, CategorySummary, ConfigLayer,
    ConfigSource, Document, Extent, Finding, FindingError, Fix, FixError, Gate, GateError,
    GenreReason, GenreResult, GenreSource, Notice, NoticeLevel, Report, RuleProvenance, RunSummary,
};
pub use schema::{
    SLUG, absent_not_null, id as schema_id, if_then, settings as schema_settings, x_extensions,
};
#[cfg(feature = "test-counters")]
#[doc(hidden)]
pub use source::indexes_built;
pub use source::{BomMode, ColumnUnit, PositionEncoding, SourceIndex};
pub use span::{LineCol, LineColError, PositionError, Span, SpanError};
pub use values::{CategoryId, Density, GenreId, KebabName, NonEmptyString, Score, ValueError};
