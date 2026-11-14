//! Safe application of fixer edits and recipe plans.
//!
//! The host validates plugin results in [`crate::hooks`] and writes nothing
//! there. This module applies them. It covers threats O6 to O9 of
//! `docs/contract/threat-model.md`:
//!
//! - **O6** A file is rewritten whole or not at all. The new bytes go to a
//!   temporary file in the same directory, and a rename puts them in place. The
//!   file must still hold the bytes the edits were computed against, checked
//!   when the file is first read and again just before the rename. The final
//!   path component is never followed (`O_NOFOLLOW` on Unix, a reparse-point
//!   check elsewhere). A file with more than one hard link is refused. Every
//!   access runs through a directory handle for the project root (`cap-std`), so
//!   a parent directory swapped for a symlink cannot lead out of the root.
//! - **O7** A recipe may not touch a path on the deny list in [`sensitive`]
//!   unless the caller approved that exact path.
//! - **O8** Plans mark agent-harness files, hooks and MCP entries so the caller
//!   can show them before it writes.
//! - **O9** Paths pass [`check_relative_path`](crate::hooks::check_relative_path)
//!   and [`resolve_in_root`](crate::hooks::resolve_in_root), then the extra
//!   checks for Windows stream and character tricks, and case-fold collisions in
//!   [`apply_plan`].
//!
//! The caller decides when writing is allowed. Nothing here prompts, and
//! nothing sets an approval itself.
//!
//! Known limits: the second source check and the rename are two steps, so a
//! writer that changes the file in between loses its update to the rename. The
//! parent directory is not synced after the rename. Windows 8.3 short names
//! such as `GIT~1` are not expanded before the deny list runs.

mod edits;
mod fs;
mod plan;
pub mod sensitive;

use std::fmt;

use thiserror::Error;

pub use edits::{EditReport, apply_edits, edited_bytes};
pub use plan::{EntryStatus, PlanEntry, PlanOptions, PlanReport, apply_plan};
pub use sensitive::HarnessWarning;

/// Why an edit set or a plan entry was refused or failed.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ApplyError {
    /// The edit set breaks the position rules or a size cap.
    #[error("edits refused: {0}")]
    InvalidEdits(String),
    /// The plan entry is incomplete.
    #[error("plan refused: {0}")]
    InvalidPlan(String),
    /// The path is not a safe relative path under the root.
    #[error("path refused: {0}")]
    PathRefused(String),
    /// The target is a symbolic link or reparse point.
    #[error("`{0}` is a symbolic link")]
    Symlink(String),
    /// The target exists and is not a regular file.
    #[error("`{0}` is not a regular file")]
    NotRegularFile(String),
    /// The target has more than one hard link.
    #[error("`{0}` has more than one hard link")]
    Hardlinked(String),
    /// The file no longer holds the bytes the edits were computed against.
    #[error("`{0}` changed since it was read")]
    SourceChanged(String),
    /// A denied path was not approved. `needs` lists the paths to approve.
    #[error("`{path}` is a sensitive path; approve {needs:?} to write it")]
    Sensitive {
        /// Path as the plan names it.
        path: String,
        /// Every path that needs an approval.
        needs: Vec<String>,
    },
    /// The directory holds an entry that differs from the path only in case.
    #[error("`{path}` collides with existing `{existing}` when case is ignored")]
    CaseCollision {
        /// Path as the plan names it.
        path: String,
        /// Name of the entry already there.
        existing: String,
    },
    /// Two plan entries name the same file.
    #[error("`{0}` appears twice in the plan")]
    DuplicatePath(String),
    /// A merge into an existing file needs a block-marker contract.
    #[error("merge into existing `{0}` is not supported yet")]
    MergeUnsupported(String),
    /// An operating-system error.
    #[error("{0}")]
    Io(String),
}

impl ApplyError {
    pub(crate) fn io(path: impl fmt::Display, e: &std::io::Error) -> Self {
        Self::Io(format!("{path}: {e}"))
    }
}

/// What an application did to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// A new file was created.
    Created,
    /// An existing file was replaced.
    Replaced,
    /// The file already held the new bytes, so nothing was written.
    Unchanged,
    /// The file was deleted.
    Removed,
    /// There was no file to delete.
    Absent,
}
