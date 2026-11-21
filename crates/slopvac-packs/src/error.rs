//! Why a pack failed to load.

use crate::problem::{ArtifactFault, LimitKind, RuleProblem, render_problems};
use crate::{ManifestError, NameError, RegistryError, SourceError};

/// A location suffix when line and column are known, else nothing.
fn at(line: Option<usize>, column: Option<usize>) -> String {
    match (line, column) {
        (Some(line), Some(column)) => format!(" (line {line}, column {column})"),
        _ => String::new(),
    }
}

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
    /// A `provides` entry is a glob that does not parse.
    #[error("`{}` is not a valid glob", slopvac_sanitize::line(.path))]
    Glob {
        /// The `provides` entry.
        path: String,
    },
    /// The category files do not load. The problems hold locations and fixed reasons only.
    #[error("the rules do not load: {}", slopvac_sanitize::line(&render_problems(.problems)))]
    Rules {
        /// Every problem found, in order.
        problems: Vec<RuleProblem>,
    },
    /// A genre or tagger file does not load. The file's own text never reaches the error.
    #[error("`{}` does not load: {fault}", slopvac_sanitize::line(.path))]
    Artifact {
        /// The pack-relative path.
        path: String,
        /// What went wrong, without file text.
        fault: ArtifactFault,
    },
    /// Two packs supply the same name.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// A genre extends itself, directly or through other genres. The error names the file of a
    /// genre on the cycle, never the genre's id.
    #[error("`{}`{}: the genre extends itself through its parents", slopvac_sanitize::line(.path), at(*.line, *.column))]
    GenreCycle {
        /// The pack-relative file of a genre on the cycle.
        path: String,
        /// The one-based line of its `extends` key.
        line: Option<usize>,
        /// The one-based column of its `extends` key.
        column: Option<usize>,
    },
    /// A genre extends a genre that no loaded pack supplies. The parent is not repeated.
    #[error("`{}`{}: the genre extends a genre that no loaded pack supplies", slopvac_sanitize::line(.path), at(*.line, *.column))]
    MissingParent {
        /// The pack-relative file of the genre.
        path: String,
        /// The one-based line of its `extends` key.
        line: Option<usize>,
        /// The one-based column of its `extends` key.
        column: Option<usize>,
    },
    /// One pack is over a size cap (X3).
    #[error("{}", crate::messages::pack_limit(.id, *.what, *.limit))]
    Limit {
        /// The pack id.
        id: String,
        /// What the cap counts.
        what: LimitKind,
        /// The cap.
        limit: u64,
    },
    /// The loaded packs together are over a size cap (X3).
    #[error("{}", crate::messages::run_limit(*.what, *.limit))]
    RunLimit {
        /// What the cap counts.
        what: LimitKind,
        /// The cap.
        limit: u64,
    },
    /// An artifact name is not valid.
    #[error(transparent)]
    Name(#[from] NameError),
}
