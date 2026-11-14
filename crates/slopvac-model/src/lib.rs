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
pub use ids::{IdError, PackId, RuleId, Severity};
pub use report::{
    AiRegister, AiRegisterSummary, AiSignal, CategoryScore, CategorySummary, ConfigSource,
    Document, Finding, Fix, Gate, GenreReason, GenreResult, GenreSource, Notice, NoticeLevel,
    Report, RuleProvenance, RunSummary, SCHEMA_VERSION,
};
pub use source::{ColumnUnit, SourceIndex};
pub use span::{LineCol, PositionError, Span};
