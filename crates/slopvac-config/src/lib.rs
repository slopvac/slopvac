//! Configuration loading and resolution.
//!
//! The crate parses `slopvac.toml` and front matter into a tree where every key
//! and value keeps its byte span, validates that tree against the v3 schemas,
//! merges the layers for one file with per-value provenance, and renders
//! failures as coded diagnostics.
#![warn(clippy::indexing_slicing, clippy::string_slice)]

mod comments;
pub mod diag;
pub mod directives;
pub mod discover;
pub mod error;
pub mod flatten;
pub mod frontmatter;
pub mod load;
pub mod presets;
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
pub use presets::{NoPresets, PackFile, Presets};
pub use resolve::{
    Explained, Inputs, LevelFrom, Resolved, ResolvedLevel, core_defaults, describe, render_explain,
};
pub use schema::{validate_config, validate_frontmatter, validate_style};
pub use source::{Layer, Setting, Source};

/// Append formatted text to `out`. A `String` takes any text, so there is no error to report.
pub(crate) fn append(out: &mut String, args: std::fmt::Arguments<'_>) {
    out.push_str(&args.to_string());
}

/// Like [`append`], then a line feed.
pub(crate) fn append_line(out: &mut String, args: std::fmt::Arguments<'_>) {
    append(out, args);
    out.push('\n');
}

/// The part of `text` in `range`. Offsets in this crate come from the parsers' spans, from
/// searches for ASCII delimiters, and from `char_indices`, so they lie on character
/// boundaries and the empty fallback does not occur.
pub(crate) fn sub<I: std::slice::SliceIndex<str, Output = str>>(text: &str, range: I) -> &str {
    text.get(range).unwrap_or_default()
}
