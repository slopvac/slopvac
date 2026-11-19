//! Why a pack failed to load.

use crate::{ManifestError, NameError, RegistryError, SourceError};

/// Why a pack failed to load. Text from pack files reaches `Display` only through
/// `slopvac_sanitize::line`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PackError {
    /// A pack file could not be read.
    #[error(transparent)]
    Source(#[from] SourceError),
    /// The manifest breaks the contract.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// The core pack's manifest names another id.
    #[error("the core pack's manifest must have the id `core`")]
    NotCore,
    /// The binary's own version is not valid semver.
    #[error("the binary's version is not valid semver")]
    BinaryVersion,
    /// A pack other than the built-in core uses the reserved id `core`.
    #[error("the id `core` is reserved for the built-in pack")]
    ReservedId,
    /// A `provides` entry is a glob, which the loader does not expand yet.
    #[error("`{}` is a glob; list each file", slopvac_sanitize::line(.path))]
    Glob {
        /// The `provides` entry.
        path: String,
    },
    /// The category files do not compile.
    #[error("the rules do not load: {}", slopvac_sanitize::line(&.0.to_string()))]
    Rules(slopvac_rules::LoadErrors),
    /// A genre or tagger file does not parse.
    #[error("`{}` does not load: {}", slopvac_sanitize::line(.path), slopvac_sanitize::line(&.source.to_string()))]
    Artifact {
        /// The pack-relative path.
        path: String,
        /// The parse failure.
        source: Box<slopvac_context::Error>,
    },
    /// Two packs supply the same name.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// An artifact name is not valid.
    #[error(transparent)]
    Name(#[from] NameError),
}
