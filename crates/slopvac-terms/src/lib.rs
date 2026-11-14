//! Project terms: term files, word blocklists, allow lists and the casing check.
//!
//! A term is a product name, language, protocol or acronym with a canonical
//! spelling. [`TermSet::check_casing`] flags case-insensitive matches that
//! differ from the canonical spelling and from every declared variant.
//! Plurals and possessives need no declaration. Blocklists carry the 2.x
//! `[[entries]]` word, part of speech and reason format.

mod blocklist;
mod casing;
mod error;
mod shape;

pub use blocklist::{Blocklist, BlocklistEntry, Pos};
pub use casing::{CasingFinding, TermSet};
pub use error::TermsError;
pub use shape::{TermEntry, TermsFile};
