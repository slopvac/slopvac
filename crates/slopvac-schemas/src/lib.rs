//! Generates the `schemas/v3` JSON Schemas from the Rust types and checks the committed files.
//!
//! [`ARTIFACTS`] lists each generated file. [`generate`] builds every file in memory: it
//! links each `$defs` entry that another file owns to that file, then writes canonical JSON
//! ([`to_canonical_json`]). [`write()`] stores the result under a repository root and
//! [`check`] compares it with the committed files.

mod artifacts;
mod canonical;
mod error;
mod files;
mod link;

pub use artifacts::{ARTIFACTS, Artifact};
pub use canonical::to_canonical_json;
pub use error::{Error, Result};
pub use files::{Drift, Generated, check, generate, generate_from, write};
pub use link::link_shared_defs;
