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
//!   path component is never followed (`cap-std`'s no-follow open on every
//!   platform, plus a check of what was opened). The temporary file starts
//!   private (mode 0600 on Unix) and takes the original mode before the rename. A file with more than one hard link is refused. Every
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
//! Known limits:
//!
//! - The second source check and the rename are two steps, so a writer that
//!   changes the file in between loses its update to the rename.
//! - The parent directory is not synced after the rename.
//! - The replacement file belongs to the user who applies it. Ownership and
//!   Windows ACLs of the original are not copied; the mode is.
//! - A plan is checked as a whole but applied file by file. It is not a
//!   transaction: after a failure the earlier files stay written and the report
//!   says which.
//! - Windows: nothing is written yet. `cap-std` rebuilds absolute paths for
//!   rename, delete and create-directory, which a junction swapped in meanwhile
//!   can redirect, and the replacement would not carry the original DACL. Recipe
//!   plans refuse every mutating entry and [`apply_edits`] refuses the call, both
//!   with [`ApplyError::UnsupportedOnWindows`]. [`edited_bytes`] and plan checks
//!   still work, so a dry run reports what would change. Names shaped like 8.3
//!   short names (`GITHUB~1`) are refused on Windows, because the deny list
//!   cannot expand them.

mod edits;
mod fs;
mod plan;
pub mod sensitive;

use std::fmt;

use slopvac_sanitize::line as esc;
use thiserror::Error;

#[doc(hidden)]
pub use edits::apply_edits_seam;
pub use edits::{EditReport, apply_edits, edited_bytes};
pub use plan::{EntryStatus, PlanEntry, PlanOptions, PlanReport, apply_plan};
#[doc(hidden)]
pub use plan::{SeamStage, apply_plan_seam};
pub use sensitive::HarnessWarning;

/// Why an edit set or a plan entry was refused or failed.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ApplyError {
    /// The edit set breaks the position rules or a size cap.
    #[error("edits refused: {}", esc(.0))]
    InvalidEdits(String),
    /// The plan entry is incomplete.
    #[error("plan refused: {}", esc(.0))]
    InvalidPlan(String),
    /// The path is not a safe relative path under the root.
    #[error("path refused: {}", esc(.0))]
    PathRefused(String),
    /// The target is a symbolic link or reparse point.
    #[error("`{}` is a symbolic link", esc(.0))]
    Symlink(String),
    /// The target exists and is not a regular file.
    #[error("`{}` is not a regular file", esc(.0))]
    NotRegularFile(String),
    /// The target has more than one hard link.
    #[error("`{}` has more than one hard link", esc(.0))]
    Hardlinked(String),
    /// The file no longer holds the bytes the edits were computed against.
    #[error("`{}` changed since it was read", esc(.0))]
    SourceChanged(String),
    /// A denied path was not approved. `needs` lists the paths to approve.
    #[error("`{}` is a sensitive path; approve {} to write it", esc(.path), quoted(.needs))]
    Sensitive {
        /// Path as the plan names it.
        path: String,
        /// Every path that needs an approval.
        needs: Vec<String>,
    },
    /// The directory holds an entry that differs from the path only in case.
    #[error("`{}` collides with existing `{}` when case is ignored", esc(.path), esc(.existing))]
    CaseCollision {
        /// Path as the plan names it.
        path: String,
        /// Name of the entry already there.
        existing: String,
    },
    /// Two plan entries name the same file.
    #[error("`{}` appears twice in the plan", esc(.0))]
    DuplicatePath(String),
    /// A merge into an existing file needs a block-marker contract.
    #[error("merge into existing `{}` is not supported yet", esc(.0))]
    MergeUnsupported(String),
    /// Writing is not available on Windows yet.
    #[error(
        "`{}`: writes are not supported on Windows yet; the file-system layer \
         resolves rename, delete and create-directory through absolute paths that a \
         junction swap can redirect",
        esc(.0)
    )]
    UnsupportedOnWindows(String),
    /// An operating-system error.
    #[error("{}", esc(.0))]
    Io(String),
}

/// ``a`, `b`` with every item escaped: untrusted paths never go through `Debug`.
fn quoted(items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|s| format!("`{}`", esc(s))).collect();
    items.join(", ")
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
