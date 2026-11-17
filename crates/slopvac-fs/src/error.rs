//! Errors for the crate.
//!
//! Display text never carries a path, so an error chain can reach a log or a report without
//! leaking where the project lives (docs/contract/policies.md section 1.7). The path stays on
//! the error: read it with [`Error::path`] and render it with [`crate::display_under_root`].

use std::io;
use std::path::{Path, PathBuf};

/// What can go wrong in this crate.
///
/// The `Debug` output is path-free too: it names the variant, the operation, the error kind and
/// the OS code, and nothing else.
#[derive(thiserror::Error)]
pub enum Error {
    /// A reader held more than the allowed number of bytes.
    #[error("input exceeds the cap of {cap} bytes")]
    CapExceeded {
        /// The cap that was exceeded.
        cap: u64,
    },
    /// No home directory is known, so no per-user directory can be chosen.
    #[error("cannot determine the home directory")]
    NoHomeDir,
    /// An I/O operation on a path failed. Read the path with [`Error::path`].
    #[error("{op} failed")]
    Io {
        /// The operation, such as `write`.
        op: &'static str,
        /// The path the operation acted on. Not part of the Display text.
        path: PathBuf,
        /// The underlying error, reduced to its OS code or kind.
        #[source]
        source: io::Error,
    },
    /// A write target was refused before anything was written: the path is not a plain relative
    /// path below the root, or the destination is a symlink or not a regular file.
    #[error("unsafe write target: {reason}")]
    UnsafeTarget {
        /// Why the target was refused.
        reason: &'static str,
        /// The path as the caller passed it. Not part of the Display text.
        path: PathBuf,
    },
    /// Reading from a reader failed.
    #[error("read failed")]
    Read(#[source] io::Error),
}

impl Error {
    /// The path the failed operation acted on, when the error has one.
    ///
    /// The path is as the caller passed it. Redact it with [`crate::display_under_root`] before
    /// it leaves the process.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Io { path, .. } | Self::UnsafeTarget { path, .. } => Some(path),
            _ => None,
        }
    }

    pub(crate) fn io(op: &'static str, path: &Path, source: &io::Error) -> Self {
        Self::Io {
            op,
            path: path.to_path_buf(),
            source: scrub(source),
        }
    }

    pub(crate) fn read(source: &io::Error) -> Self {
        Self::Read(scrub(source))
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CapExceeded { cap } => f.debug_struct("CapExceeded").field("cap", cap).finish(),
            Self::NoHomeDir => f.write_str("NoHomeDir"),
            Self::Io { op, source, .. } => f
                .debug_struct("Io")
                .field("op", op)
                .field("kind", &source.kind())
                .field("os_code", &source.raw_os_error())
                .finish_non_exhaustive(),
            Self::UnsafeTarget { reason, .. } => f
                .debug_struct("UnsafeTarget")
                .field("reason", reason)
                .finish_non_exhaustive(),
            Self::Read(source) => f
                .debug_struct("Read")
                .field("kind", &source.kind())
                .field("os_code", &source.raw_os_error())
                .finish(),
        }
    }
}

/// Keep the OS code or the kind of `e` and drop its message and inner error.
///
/// Libraries such as `tempfile` wrap an I/O error together with the path it concerned, and
/// that path would appear in the chain.
fn scrub(e: &io::Error) -> io::Error {
    e.raw_os_error()
        .map_or_else(|| io::Error::from(e.kind()), io::Error::from_raw_os_error)
}

/// A result with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
