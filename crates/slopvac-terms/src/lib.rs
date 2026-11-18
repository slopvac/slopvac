//! Project terms: term files, word blocklists, allow lists and the casing check.
//!
//! A term is a product name, language, protocol or acronym with a canonical
//! spelling. [`TermSet::check_casing`] flags case-insensitive matches that
//! differ from the canonical spelling and from every declared variant.
//! Plurals and possessives need no declaration. Blocklists use `[[entries]]`
//! tables of word, part of speech and reason, under `schema_version = 3`.
//! [`propernoun`] infers candidate names from body prose, for `terms suggest`
//! and heading checks.

mod blocklist;
mod casing;
pub mod cli;
mod edit;
mod error;
pub mod import;
pub mod propernoun;
mod read;
mod reference;
mod shape;
pub mod suggest;

pub use blocklist::{Blocklist, BlocklistEntry, Pos};
pub use casing::{CasingFinding, TermSet};
pub use edit::{TermsDoc, infer_ignore_case};
pub use error::{EntryProblem, TermsError};
pub use read::MAX_TERMS_FILE_BYTES;
pub use reference::reference_pages;
pub use shape::{TermEntry, TermsFile};
