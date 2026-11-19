//! The messages `slopvac self update` reports when the update lock or the installed binary
//! stops an update.
//!
//! Each variant is one stable code. [`UpdateMessage::entries`] lists them for the workspace
//! message registry, which generates the reference page and checks that no code changes.

use slopvac_messages::Catalog;

/// Why the update lock or the installed binary refused an update.
#[derive(Debug, Catalog)]
pub enum UpdateMessage {
    /// Another `self update` held the update lock for the whole 60-second wait.
    #[code("update-in-progress")]
    #[message("another slopvac update is running (lock file {file}); try again when it finishes")]
    InProgress {
        /// Lock file name.
        file: String,
    },
    /// The update lock file is a symbolic link, has more than one hard link, belongs to another user or has a mode looser than 0600.
    #[code("update-lock-unsafe")]
    #[message("refusing to use the update lock {file}: {problem}")]
    LockUnsafe {
        /// Lock file name.
        file: String,
        /// What is wrong with it.
        problem: String,
    },
    /// An earlier update died part-way and the installed binary matches no known build, or the update record is unreadable.
    #[code("update-interrupted")]
    #[message(
        "an earlier update was interrupted and the installed binary is not one it knows; \
         check the installed slopvac. {hint}"
    )]
    Interrupted {
        /// What to do next, which depends on whether another update holds the lock.
        hint: String,
    },
    /// No update record vouches for the installed binary and it did not report a version when run.
    #[code("update-version-unknown")]
    #[message(
        "cannot tell which version of slopvac is installed ({reason}); refusing to install over \
         it. Check that the installed slopvac runs, then run the update again"
    )]
    VersionUnknown {
        /// Why the executable's version is unknown.
        reason: String,
    },
}
