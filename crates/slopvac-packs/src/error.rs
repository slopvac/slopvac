//! Why a pack failed to load.

use crate::problem::{ArtifactFault, RuleProblem, render_problems};
use crate::{ManifestError, NameError, RegistryError, SourceError};

/// A location suffix when line and column are known, else nothing.
fn at(line: Option<usize>, column: Option<usize>) -> String {
    match (line, column) {
        (Some(line), Some(column)) => format!(" (line {line}, column {column})"),
        _ => String::new(),
    }
}

impl From<slopvac_rules::BudgetHit> for PackError {
    fn from(hit: slopvac_rules::BudgetHit) -> Self {
        Self::Budget {
            budget: hit.budget,
            limit: hit.limit,
        }
    }
}

impl PackError {
    /// The budget this refusal crossed and its limit, which are the `data.budget` and
    /// `data.limit` of a `rules-budget-exceeded` report. `None` for any other error.
    #[must_use]
    pub const fn budget(&self) -> Option<(slopvac_rules::PackBudget, u64)> {
        match self {
            Self::Budget { budget, limit } => Some((*budget, *limit)),
            _ => None,
        }
    }

    /// The stable code of this error, for the variants that have one. A wrapped error reports
    /// its own code.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        use crate::messages::code_of;
        match self {
            Self::Source(e) => e.code(),
            Self::Registry(e) => e.code(),
            Self::Budget { budget, .. } => Some(budget.code()),
            Self::GlobTooComplex => Some(code_of!(GlobTooComplex {
                depth,
                alternatives,
                bytes
            })),
            _ => None,
        }
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
    /// A `provides` glob is over the limits of the loader (X3).
    #[error(
        "{}",
        crate::messages::glob_too_complex(
            crate::GLOB_MAX_DEPTH,
            crate::GLOB_MAX_ALTERNATIVES,
            crate::GLOB_MAX_BYTES
        )
    )]
    GlobTooComplex,
    /// A pack or the run is over a budget (X3). The code is `rules-budget-exceeded`, with
    /// `data.budget` the name of the budget (`plugin-files`, `plugin-bytes`, `plugin-run-bytes`,
    /// `plugin-run-rules` or `plugin-run-pattern-bytes`) and `data.limit` the limit.
    #[error("{}", .budget.message(*.limit))]
    Budget {
        /// The budget that was crossed.
        budget: slopvac_rules::PackBudget,
        /// Its limit, in the unit of the budget.
        limit: u64,
    },
    /// An artifact name is not valid.
    #[error(transparent)]
    Name(#[from] NameError),
}
