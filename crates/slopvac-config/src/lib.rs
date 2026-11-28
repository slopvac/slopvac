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
mod wire;

pub use diag::{BudgetData, Code, ConfigBudget, Diagnostic, SourceFile, render};
pub use discover::Discovery;
pub use error::{ConfigError, Rejected};
pub use files::{FilesError, FilesMatcher};
pub use load::{LoadedConfig, load, load_labeled, load_text};
pub use messages::{document_size_message, error_entries, phrase_entries, text_entries};
pub use presets::{PackFile, PresetFailure, Presets};
pub use read::{
    MAX_CONFIG_BYTES, MAX_DOCUMENT_BYTES, MAX_EXTENDS_APPLIED_BYTES, MAX_EXTENDS_BYTES,
    MAX_EXTENDS_DEPTH, MAX_EXTENDS_FILES, MAX_FILE_BLOCKS, MAX_INDEX_EXTRA,
};
pub use resolve::{
    ConfigLevel, Contribution, Explained, GroupedKey, Inputs, LevelFrom, Resolved, ResolvedLevel,
    core_defaults, describe, render_explain,
};
pub use schema::{validate_config, validate_frontmatter, validate_style};
pub use slopvac_model::{ConfigLayer, ConfigSource, Severity};
pub use source::{Setting, bare_source, layer_name, located_source};
pub use unknown_rule::{UnknownRule, unknown_rules};
pub use wire::{config_schema, frontmatter_schema, style_schema};

/// The levels a rule or category reports at: `error`, `warning`, `suggestion` and `off`. This
/// is the one list for the config and style readers and for the `RuleLevel` definition in
/// `config.schema.json`. `slopvac-styles` holds its `Level` enum equal to it by a test.
pub const RULE_LEVELS: &[&str] = schema::LEVELS;

/// The category density budget that clears a budget a lower layer set. This is the one source
/// for the config and style readers and for the budget definition in the schemas.
pub const BUDGET_NONE: &str = schema::BUDGET_NONE;

/// The least value of a density budget. It also bounds a count and a weight. The bound is
/// inclusive: the value itself is accepted. This is the one source for the config and style
/// readers and for the `minimum` keyword of the schemas.
pub const BUDGET_MINIMUM: u32 = schema::MINIMUM;

/// Whether `s` is a plugin, category, genre or context-tag name: a lower-case letter, then
/// lower-case letters, digits and hyphens. This is the one check for the config reader and
/// every other reader of such a name, and the property test in `schema/patterns.rs` holds it
/// to the `^[a-z][a-z0-9-]*` pattern that the schemas publish.
#[must_use]
pub fn is_name(s: &str) -> bool {
    schema::is_pack_id(s)
}

/// Whether `s` is a `Ref` of `config.schema.json`: a style or genre name, bare or qualified as
/// `<plugin>:<name>`. This is the one check for every reader of such a name.
#[must_use]
pub fn is_ref(s: &str) -> bool {
    schema::is_ref(s)
}

/// Whether `s` is a `RuleKey` of `config.schema.json`: a rule or category name, bare or
/// qualified as `<plugin>:<id>`. This is the one check for every reader of such a name.
#[must_use]
pub fn is_rule_key(s: &str) -> bool {
    schema::is_rule_key(s)
}
