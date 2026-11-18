//! Configuration loading and resolution.
//!
//! The crate parses `slopvac.toml` and front matter into a tree where every key
//! and value keeps its byte span, validates that tree against the v3 schemas,
//! merges the layers for one file with per-value provenance, and renders
//! failures as coded diagnostics.

mod comments;
pub mod diag;
pub mod directives;
pub mod discover;
pub mod error;
pub mod flatten;
pub mod frontmatter;
pub mod load;
pub mod presets;
mod read;
pub mod resolve;
pub mod schema;
pub mod source;
pub mod tree;
pub mod validate;
pub mod version;

pub use diag::{Code, Diagnostic, SourceFile, render};
pub use discover::Discovery;
pub use error::{ConfigError, Rejected};
pub use load::{LoadedConfig, load, load_labeled};
pub use presets::{NoPresets, PackFile, PresetFailure, Presets};
pub use read::{
    MAX_CONFIG_BYTES, MAX_DOCUMENT_BYTES, MAX_EXTENDS_BYTES, MAX_EXTENDS_DEPTH, MAX_EXTENDS_FILES,
};
pub use resolve::{
    Explained, Inputs, LevelFrom, Resolved, ResolvedLevel, core_defaults, describe, render_explain,
};
pub use schema::{validate_config, validate_frontmatter, validate_style};
pub use source::{Layer, Setting, Source};
