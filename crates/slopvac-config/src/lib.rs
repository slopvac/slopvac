//! Configuration loading and resolution.
//!
//! The crate parses `slopvac.toml` and front matter into a tree where every key
//! and value keeps its byte span, validates that tree against the v3 schemas,
//! and renders failures as coded diagnostics.

pub mod diag;
pub mod schema;
pub mod tree;

pub use diag::{Code, Diagnostic, SourceFile, render};
pub use schema::{validate_config, validate_frontmatter};
