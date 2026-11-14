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
mod plan;
mod renames;
mod report;
mod steering;

use std::fmt;

pub use cli::{command, run};
pub use config::{Rewrite, STE_STYLE, migrate_config};
pub use directives::rewrite_directives;
pub use plan::{BACKUP_DIR, Change, ChangeKind, Plan, apply, plan, render_diff, restore};
pub use renames::RenameTable;
pub use report::{Finding, Linter, NullLinter, Phase, RuleDelta, compare};
pub use steering::rewrite_steering;

/// Switches for a migration run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Keep the 2.x vocabulary behavior by adding the STE style.
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
            "config-legacy-dropped: {}:{}: `{}`: {}",
            self.file, self.line, self.key, self.reason
        )
    }
}

/// Failures of a migration.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Settings with no 3.0 equivalent. Nothing is written.
    #[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
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
    pub(crate) fn io(path: &std::path::Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            source,
        }
    }
}
