//! Errors for the crate.

use std::path::PathBuf;

/// What can go wrong in this crate.
#[derive(Debug, thiserror::Error)]
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
    /// An I/O operation failed. The path is the one the caller passed.
    #[error("{op} {path}: {source}")]
    Io {
        /// The operation, such as `write`.
        op: &'static str,
        /// The path the operation acted on.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Reading from a reader failed.
    #[error("read failed: {0}")]
    Read(#[source] std::io::Error),
}

/// A result with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
