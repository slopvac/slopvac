//! Errors for the crate.
//!
//! Display text never carries a path, so an error chain can reach a log or a report without
//! leaking where the project lives (docs/contract/policies.md section 1.7). The path stays on
//! the error: read it with [`Error::path`] and render it with [`crate::display_under_root`].

use std::io;
use std::path::{Path, PathBuf};

/// What can go wrong in this crate.
///
/// Callers cannot build an error with a path or an unscrubbed source: the variants that carry
/// them are `#[non_exhaustive]`, and only this crate constructs them. Match with `..`.
///
/// The `Debug` output is path-free too: it names the variant, the operation, the error kind and
/// the OS code, and nothing else.
#[derive(thiserror::Error)]
#[non_exhaustive]
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
    /// An environment variable that names a directory is not valid UTF-8. Directories must be
    /// valid UTF-8, because they come from configuration and output that must be UTF-8.
    #[error("environment variable {var} is not valid UTF-8")]
    NotUtf8Env {
        /// The variable name. The value is never kept.
        var: &'static str,
    },
    /// An I/O operation on a path failed. Read the path with [`Error::path`].
    #[error("{op} failed")]
    #[non_exhaustive]
    Io {
        /// The operation that failed.
        op: Op,
        /// The path the operation acted on. Not part of the Display text.
        path: PathBuf,
        /// The underlying error, reduced to its OS code or kind.
        #[source]
        source: io::Error,
    },
    /// A target was refused before anything was read or written: the path is not a plain
    /// relative path below the root, or a write destination is a symlink or not a regular file.
    #[error("unsafe target: {reason}")]
    #[non_exhaustive]
    UnsafeTarget {
        /// Why the target was refused.
        reason: Reason,
        /// The path as the caller passed it. Not part of the Display text.
        path: PathBuf,
    },
    /// A file read by path did not hold valid UTF-8. Read the path with [`Error::path`].
    #[error("not valid UTF-8")]
    #[non_exhaustive]
    NotUtf8 {
        /// The path that was read. Not part of the Display text.
        path: PathBuf,
    },
    /// A confined file read was refused or failed. Match on `kind`.
    #[error("input refused: {kind}")]
    #[non_exhaustive]
    Input {
        /// What went wrong.
        kind: InputKind,
        /// The path as the caller passed it. Not part of the Display text.
        path: PathBuf,
        /// The scrubbed I/O error, present for [`InputKind::Io`] only.
        #[source]
        source: Option<io::Error>,
    },
    /// Reading from a reader failed.
    #[error("read failed")]
    Read(#[source] io::Error),
}

/// The operation an [`Error::Io`] failed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Op {
    /// Opening or creating the directory that holds the destination.
    OpenDirectory,
    /// Reading the destination's metadata.
    Stat,
    /// Creating the temporary file.
    CreateTemp,
    /// Writing, setting permissions on or syncing the temporary file.
    Write,
    /// Checking that the temporary file is still the one that was created.
    VerifyStaged,
    /// Renaming the temporary file over the destination.
    Replace,
    /// Syncing the destination's directory.
    SyncDirectory,
    /// Resolving the directory of a path for reading.
    ResolveParent,
    /// Opening a file to read it.
    Open,
    /// Reading from an open file.
    ReadFile,
}

impl Op {
    /// A short phrase for the operation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenDirectory => "open directory of",
            Self::Stat => "stat",
            Self::CreateTemp => "create temporary file for",
            Self::Write => "write",
            Self::VerifyStaged => "verify staged file of",
            Self::Replace => "replace",
            Self::SyncDirectory => "sync directory of",
            Self::ResolveParent => "open parent of",
            Self::Open => "open",
            Self::ReadFile => "read",
        }
    }
}

impl std::fmt::Display for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an [`Error::UnsafeTarget`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reason {
    /// The path is absolute or has a `..` component.
    NotRelative,
    /// The path names no file.
    NoFileName,
    /// The write destination is a symlink.
    DestinationSymlink,
    /// The write destination exists and is not a regular file.
    DestinationNotRegular,
}

impl Reason {
    /// A short phrase for the reason.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRelative => "path must be relative and stay below the root",
            Self::NoFileName => "path names no file",
            Self::DestinationSymlink => "destination is a symlink",
            Self::DestinationNotRegular => "destination is not a regular file",
        }
    }
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an [`Error::Input`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputKind {
    /// The path is absolute, has a `..` component or names no file, so it was not tried.
    UnsafePath,
    /// The file, or a directory on the way to it, does not exist. Callers may treat this as
    /// normal.
    NotFound,
    /// A component of the path, the last included, is a symlink.
    Symlink,
    /// The target is not a regular file: a directory, FIFO, socket or device.
    NotRegular,
    /// The file holds more than the cap.
    TooLarge,
    /// The file is not valid UTF-8. Only the `_to_string` readers report this.
    NotUtf8,
    /// The file has more than one hard link.
    MultiplyLinked,
    /// Any other I/O failure. The scrubbed error is the source.
    Io,
}

impl InputKind {
    /// A short phrase for the kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsafePath => "path must be relative, stay below the root and name a file",
            Self::NotFound => "not found",
            Self::Symlink => "a path component is a symlink",
            Self::NotRegular => "not a regular file",
            Self::TooLarge => "larger than the cap",
            Self::NotUtf8 => "not valid UTF-8",
            Self::MultiplyLinked => "has more than one hard link",
            Self::Io => "I/O error",
        }
    }
}

impl std::fmt::Display for InputKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Error {
    /// The path the failed operation acted on, when the error has one.
    ///
    /// The path is as the caller passed it. Redact it with [`crate::display_under_root`] before
    /// it leaves the process.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Io { path, .. }
            | Self::UnsafeTarget { path, .. }
            | Self::NotUtf8 { path, .. }
            | Self::Input { path, .. } => Some(path),
            _ => None,
        }
    }

    /// The kind of a confined read failure, when this is one.
    #[must_use]
    pub fn input_kind(&self) -> Option<InputKind> {
        match self {
            Self::Input { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    pub(crate) fn io(op: Op, path: &Path, source: &io::Error) -> Self {
        Self::Io {
            op,
            path: path.to_path_buf(),
            source: scrub(source),
        }
    }

    pub(crate) fn unsafe_target(reason: Reason, path: &Path) -> Self {
        Self::UnsafeTarget {
            reason,
            path: path.to_path_buf(),
        }
    }

    pub(crate) fn input(kind: InputKind, path: &Path, source: Option<&io::Error>) -> Self {
        Self::Input {
            kind,
            path: path.to_path_buf(),
            source: source.map(scrub),
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
            Self::NotUtf8Env { var } => f.debug_struct("NotUtf8Env").field("var", var).finish(),
            Self::Input { kind, source, .. } => f
                .debug_struct("Input")
                .field("kind", kind)
                .field(
                    "os_code",
                    &source.as_ref().and_then(io::Error::raw_os_error),
                )
                .finish_non_exhaustive(),
            Self::Io { op, source, .. } => f
                .debug_struct("Io")
                .field("op", &op.as_str())
                .field("kind", &source.kind())
                .field("os_code", &source.raw_os_error())
                .finish_non_exhaustive(),
            Self::UnsafeTarget { reason, .. } => f
                .debug_struct("UnsafeTarget")
                .field("reason", &reason.as_str())
                .finish_non_exhaustive(),
            Self::NotUtf8 { .. } => f.write_str("NotUtf8"),
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
