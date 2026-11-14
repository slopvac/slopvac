//! Plugin discovery and execution.
//!
//! The [`lock`] module holds the `.slopvac/packs.lock` model, its strict
//! reader, its canonical writer, the drift diff and the `outdated` query.

pub mod lock;
