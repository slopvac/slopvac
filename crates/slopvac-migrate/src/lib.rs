//! `slopvac migrate`: move a 2.x project to 3.0.
//!
//! 3.0 has no compatibility mode. This crate is the only path from 2.x. It rewrites
//! configuration (comments kept), inline directive ids and steering blocks, and
//! compares findings before and after.
//!
//! The CLI crate wires [`command`] and [`run`] into the clap tree.

mod cli;
mod config;
mod directives;
mod guide;
mod legacy;
pub mod overlap;
mod plan;
mod preserve;
mod preserve_doc;
mod renames;
mod report;
mod steering;

use std::fmt;

pub use cli::{command, execute, run};
pub use config::{Rewrite, migrate_config};
pub use directives::{UnknownId, rewrite_directives};
pub use guide::{doc as guide_doc, prime_migrate};
pub use plan::{BACKUP_DIR, Change, ChangeKind, Plan, apply, plan, render_diff, restore};
pub use renames::RenameTable;
pub use report::{Finding, Linter, NullLinter, Phase, RuleDelta, compare};
pub use steering::rewrite_steering;

/// Switches for a migration run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Reproduce the 2.x profile gates: select the `core:legacy-<profile>` style and
    /// expand the config (docs/contract/profiles.md, section 4).
    pub preserve_behavior: bool,
}

/// A 2.x setting that 3.0 cannot represent (`config-legacy-dropped`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    /// The file that holds the setting.
    pub file: String,
    /// The key path, such as `vale.binary` or `overrides[0].guidance`.
    pub key: String,
    /// One-based line of the key.
    pub line: usize,
    /// Why 3.0 cannot take it.
    pub reason: String,
}

impl fmt::Display for Dropped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: `{}`: {}",
            self.file, self.line, self.key, self.reason
        )
    }
}

/// Failures of a migration.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Settings with no 3.0 equivalent. Nothing is written.
    #[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n  "))]
    Dropped(Vec<Dropped>),
    /// The file is not valid TOML.
    #[error("{file}: {message}")]
    Parse {
        /// The file.
        file: String,
        /// The parser message.
        message: String,
    },
    /// The config already carries `schema_version = 3`.
    #[error("{file}: already migrated (schema_version is 3 or higher)")]
    AlreadyMigrated {
        /// The file.
        file: String,
    },
    /// Two config files in one directory make the result ambiguous.
    #[error("{0}")]
    Conflict(String),
    /// A path would leave the project or cross a symbolic link, or a backup is in the way.
    #[error("unsafe: {0}")]
    Unsafe(String),
    /// `--preserve-behavior` cannot decide how an override block overlaps the
    /// others, because its `files` pattern is outside the supported grammar.
    #[error(
        "{file}:{line}: `overrides[{block}]`: cannot decide how this block overlaps the others: {reason}. \
         --preserve-behavior accepts one pattern per block: an anchored path such as `docs/api` \
         or a tree such as `docs/**`"
    )]
    OverrideOverlap {
        /// The file.
        file: String,
        /// One-based line of the block.
        line: usize,
        /// Index of the block in `overrides`.
        block: usize,
        /// What is wrong with the pattern.
        reason: String,
    },
    /// The rename table is malformed.
    #[error("rename table: {0}")]
    RenameTable(String),
    /// A filesystem operation failed.
    #[error("{path}: {source}")]
    Io {
        /// The path.
        path: String,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    /// The stable error code (docs/contract/policies.md, section 3.5).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Dropped(_) => "config-legacy-dropped",
            Self::Parse { .. } => "config-invalid-toml",
            Self::AlreadyMigrated { .. } => "config-already-migrated",
            Self::Conflict(_) => "migrate-config-conflict",
            Self::Unsafe(_) => "migrate-unsafe-path",
            Self::OverrideOverlap { .. } => "migrate-override-overlap",
            Self::RenameTable(_) => "migrate-rename-table",
            Self::Io { .. } => "migrate-io-failed",
        }
    }

    /// Show a path outside `root` as `<external>/<file name>` and one inside as
    /// relative to it, so messages never carry absolute paths (policies, 1.7).
    #[must_use]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "only Io carries a path; every other error passes through unchanged"
    )]
    pub(crate) fn redacted(self, root: &std::path::Path) -> Self {
        match self {
            Self::Io { path, source } => {
                let p = std::path::Path::new(&path);
                let shown = match p.strip_prefix(root) {
                    Ok(rel) if rel.as_os_str().is_empty() => ".".to_owned(),
                    Ok(rel) => rel.display().to_string(),
                    Err(_) => format!(
                        "<external>/{}",
                        p.file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default()
                    ),
                };
                Self::Io {
                    path: shown,
                    source,
                }
            }
            other => other,
        }
    }

    pub(crate) fn io(path: &std::path::Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            source,
        }
    }
}
