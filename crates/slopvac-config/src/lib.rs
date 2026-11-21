//! Configuration loading and resolution.
//!
//! The crate parses `slopvac.toml` and front matter into a tree where every key
//! and value keeps its byte span, validates that tree against the v3 schemas,
//! merges the layers for one file with per-value provenance, and renders
//! failures as coded diagnostics.
#![allow(
    clippy::allow_attributes_without_reason,
    reason = "TEMPORARY: burn-down in slopvac-8l4.2.32"
)]

mod comments;
pub mod diag;
pub mod directives;
pub mod discover;
pub mod error;
pub mod files;
pub mod flatten;
pub mod frontmatter;
pub mod load;
mod messages;
pub mod presets;
mod read;
pub mod resolve;
pub mod schema;
pub mod source;
pub mod tree;
mod unknown_rule;
pub mod validate;
pub mod version;

pub use diag::{BudgetData, Code, ConfigBudget, Diagnostic, SourceFile, render};
pub use discover::Discovery;
pub use error::{ConfigError, Rejected};
pub use files::{FilesError, FilesMatcher};
pub use load::{LoadedConfig, load, load_labeled};
pub use messages::{document_size_message, error_entries, phrase_entries, text_entries};
pub use presets::{PackFile, PresetFailure, Presets};
pub use read::{
    MAX_CONFIG_BYTES, MAX_DOCUMENT_BYTES, MAX_EXTENDS_APPLIED_BYTES, MAX_EXTENDS_BYTES,
    MAX_EXTENDS_DEPTH, MAX_EXTENDS_FILES, MAX_FILE_BLOCKS, MAX_INDEX_EXTRA,
};
pub use resolve::{
    ConfigLevel, Explained, Inputs, LevelFrom, Resolved, ResolvedLevel, core_defaults, describe,
    render_explain,
};
pub use schema::{validate_config, validate_frontmatter, validate_style};
pub use slopvac_model::{ConfigLayer, ConfigSource, Severity};
pub use source::{Setting, bare_source, layer_name, located_source};
pub use unknown_rule::{UnknownRule, unknown_rules};
