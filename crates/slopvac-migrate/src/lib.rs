//! `slopvac migrate`: move a 2.x project to 3.0.
//!
//! 3.0 has no compatibility mode. This crate is the only path from 2.x. It rewrites
//! configuration (comments kept), inline directive ids and steering blocks, and
//! compares findings before and after.
//!
//! The CLI crate wires [`command`] and [`run`] into the clap tree.

mod backup;
mod blocklist;
mod capped;
mod cli;
mod config;
mod directives;
mod guide;
mod legacy;
mod limits;
mod lint;
pub mod overlap;
mod plan;
mod preserve;
mod preserve_doc;
mod renames;
mod report;
mod steering;
mod walk;

use std::fmt;

use slopvac_config::Code;

pub use backup::{Restored, apply, apply_hooked, restore, restore_hooked};
pub use cli::{CliFailure, command, execute, run};
pub use config::{Rewrite, migrate_config};
pub use directives::{Found, RemovedId, UnknownId, rewrite_directives};
pub use guide::{doc as guide_doc, prime_migrate};
pub use limits::Limits;
pub use lint::EngineLinter;
pub use plan::{BACKUP_DIR, Change, ChangeKind, Plan, plan, plan_with_limits, render_diff};
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
///
/// A run keeps a bounded number of these. When it left some out, the last record has a
/// non-zero `suppressed` count and a `reason` that states it.
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
    /// Zero for a real setting. For the closing record of a capped list, how many settings
    /// the run left out.
    pub suppressed: usize,
}

impl Dropped {
    /// A setting at `line` of `file`.
    #[must_use]
    pub fn new(
        file: impl Into<String>,
        key: impl Into<String>,
        line: usize,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            file: file.into(),
            key: key.into(),
            line,
            reason: reason.into(),
            suppressed: 0,
        }
    }

    /// The closing record of a capped list, for `count` settings left out.
    #[must_use]
    pub fn suppression(count: usize) -> Self {
        Self {
            file: String::new(),
            key: String::new(),
            line: 0,
            reason: format!("{count} more blocking settings suppressed"),
            suppressed: count,
        }
    }

    /// The setting as a notice with the `migrate-category-removed` code, for a
    /// category that 3.0 no longer has. Unlike a blocking drop, it does not stop the
    /// migration.
    #[must_use]
    pub fn category_removed_notice(&self) -> String {
        format!("migrate-category-removed: {self}")
    }
}

impl fmt::Display for Dropped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The closing record of a capped list only counts the rest.
        if self.suppressed > 0 {
            return f.write_str(&self.reason);
        }
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
    /// A path would leave the project, cross a symbolic link or share its inode, or a backup
    /// is in the way or was not written by this tool.
    #[error("unsafe: {0}")]
    Unsafe(String),
    /// A file changed after the migration was planned, or after it was written. Nothing
    /// more is written.
    #[error("changed: {0}")]
    Changed(String),
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
    /// The engine could not lint the project.
    #[error("lint: {0}")]
    Lint(String),
    /// `schema_version` is not an integer.
    #[error("{file}:{line}: {message}")]
    InvalidType {
        /// The file.
        file: String,
        /// One-based line of the value.
        line: usize,
        /// What is wrong.
        message: String,
    },
    /// Reading a file failed.
    #[error("{path}: {source}")]
    Read {
        /// The file.
        path: String,
        /// The cause.
        #[source]
        source: slopvac_fs::Error,
    },
    /// A document is larger than the document cap.
    #[error("{path}: larger than {cap} bytes")]
    DocumentTooLarge {
        /// The file.
        path: String,
        /// The cap in bytes.
        cap: u64,
    },
    /// A config file is larger than the config cap.
    #[error("{path}: larger than {cap} bytes")]
    ConfigTooLarge {
        /// The file.
        path: String,
        /// The cap in bytes.
        cap: u64,
    },
    /// A blocklist is larger than the terms file cap.
    #[error("{path}: larger than {cap} bytes")]
    TermsFileTooLarge {
        /// The file.
        path: String,
        /// The cap in bytes.
        cap: u64,
    },
    /// A run read more files or bytes than its limits allow.
    #[error("{path}: run limit reached: more than {limit} {what} in one run")]
    RunLimit {
        /// The file that crossed the limit.
        path: String,
        /// What was counted: `files` or `bytes`.
        what: &'static str,
        /// The limit.
        limit: u64,
    },
    /// A file is not valid UTF-8.
    #[error("{path}: not valid UTF-8")]
    NotUtf8 {
        /// The file.
        path: String,
    },
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
            Self::InvalidType { .. } => "config-invalid-type",
            Self::AlreadyMigrated { .. } => "config-already-migrated",
            Self::Conflict(_) => "migrate-config-conflict",
            Self::Unsafe(_) => "migrate-unsafe-path",
            Self::Changed(_) => "migrate-file-changed",
            Self::OverrideOverlap { .. } => "migrate-override-overlap",
            Self::RenameTable(_) => "migrate-rename-table",
            Self::Lint(_) => "migrate-lint-failed",
            Self::DocumentTooLarge { .. } => Code::DocumentTooLarge.as_str(),
            Self::ConfigTooLarge { .. } => Code::TooLarge.as_str(),
            Self::TermsFileTooLarge { .. } => "terms-file-too-large",
            Self::RunLimit { .. } => "migrate-run-limit",
            Self::NotUtf8 { .. } => "migrate-not-utf8",
            Self::Io { .. } | Self::Read { .. } => "migrate-io-failed",
        }
    }

    /// Show a path outside `root` as `<external>/<file name>` and one inside as
    /// relative to it, so messages never carry absolute paths (policies, 1.7).
    #[must_use]
    pub(crate) fn redacted(self, root: &std::path::Path) -> Self {
        match self {
            Self::Io { path, source } => Self::Io {
                path: shown(&path, root),
                source,
            },
            Self::DocumentTooLarge { path, cap } => Self::DocumentTooLarge {
                path: shown(&path, root),
                cap,
            },
            Self::ConfigTooLarge { path, cap } => Self::ConfigTooLarge {
                path: shown(&path, root),
                cap,
            },
            Self::TermsFileTooLarge { path, cap } => Self::TermsFileTooLarge {
                path: shown(&path, root),
                cap,
            },
            Self::RunLimit { path, what, limit } => Self::RunLimit {
                path: shown(&path, root),
                what,
                limit,
            },
            Self::NotUtf8 { path } => Self::NotUtf8 {
                path: shown(&path, root),
            },
            Self::Read { path, source } => Self::Read {
                path: shown(&path, root),
                source,
            },
            other => other,
        }
    }

    /// The same failure, reported for `shown`.
    pub(crate) fn at(self, shown: &std::path::Path) -> Self {
        let path = shown.display().to_string();
        match self {
            Self::DocumentTooLarge { cap, .. } => Self::DocumentTooLarge { path, cap },
            Self::ConfigTooLarge { cap, .. } => Self::ConfigTooLarge { path, cap },
            Self::TermsFileTooLarge { cap, .. } => Self::TermsFileTooLarge { path, cap },
            Self::RunLimit { what, limit, .. } => Self::RunLimit { path, what, limit },
            Self::NotUtf8 { .. } => Self::NotUtf8 { path },
            Self::Read { source, .. } => Self::Read { path, source },
            Self::Io { source, .. } => Self::Io { path, source },
            other => other,
        }
    }

    pub(crate) fn read(path: &std::path::Path, source: slopvac_fs::Error) -> Self {
        Self::Read {
            path: path.display().to_string(),
            source,
        }
    }

    pub(crate) fn io(path: &std::path::Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            source,
        }
    }
}

/// `path` relative to `root`, or `<external>/<file name>` outside it.
fn shown(path: &str, root: &std::path::Path) -> String {
    let p = std::path::Path::new(path);
    match p.strip_prefix(root) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_owned(),
        Ok(rel) => rel.display().to_string(),
        Err(_) => format!(
            "<external>/{}",
            p.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ),
    }
}
