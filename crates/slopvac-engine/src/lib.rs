//! The linting engine that applies rules to documents.
//!
//! File discovery lives in [`discover`]. [`rst`] turns reStructuredText into the same
//! blocks and projections the other parsers produce.

pub mod discover;
pub mod rst;
