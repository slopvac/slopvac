//! Core types shared by every slopvac crate.
//!
//! The shapes match `schemas/v3`. Positions follow `docs/contract/positions.md`: a
//! [`Span`] of byte offsets is the only stored position, and [`SourceIndex`] derives
//! line and column from it.

mod edit;
mod ids;
mod report;
mod source;
mod span;

pub use edit::{Edit, EditError, apply_edits};
pub use ids::{ApiVersion, ContextTag, IdError, PackId, RuleId, SchemaVersion, Severity};
pub use report::{
    AiRegister, AiRegisterSummary, AiSignal, CategoryScore, CategorySummary, ConfigSource,
    Document, Finding, Fix, Gate, GenreReason, GenreResult, GenreSource, Notice, NoticeLevel,
    Report, RuleProvenance, RunSummary,
};
pub use source::{BomMode, ColumnUnit, PositionEncoding, SourceIndex};
pub use span::{LineCol, LineColError, PositionError, Span, SpanError};
