//! Project terms: term files, word blocklists, allow lists and the casing check.
//!
//! A term is a product name, language, protocol or acronym with a canonical
//! spelling. [`TermSet::check_casing`] flags case-insensitive matches that
//! differ from the canonical spelling and from every declared variant.
//! Plurals and possessives need no declaration. Blocklists carry the 2.x
//! `[[entries]]` word, part of speech and reason format.
#![warn(clippy::indexing_slicing, clippy::string_slice)]

mod blocklist;
mod casing;
pub mod cli;
mod edit;
mod error;
pub mod import;
mod reference;
mod shape;
pub mod suggest;

pub use blocklist::{Blocklist, BlocklistEntry, Pos};
pub use casing::{CasingFinding, TermSet};
pub use edit::{TermsDoc, infer_ignore_case};
pub use error::TermsError;
pub use reference::reference_pages;
pub use shape::{TermEntry, TermsFile};

/// Drop the result of a write into a `String`. `fmt::Write for String` never returns an
/// error, so there is nothing to handle; this is the one place that says so.
#[expect(
    clippy::let_underscore_must_use,
    reason = "fmt::Write for String never returns an error"
)]
pub(crate) fn infallible(result: std::fmt::Result) {
    let _ = result;
}
