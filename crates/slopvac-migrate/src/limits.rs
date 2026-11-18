//! Input caps and per-run limits for everything migrate reads.

use std::fs::File;
use std::path::Path;

use slopvac_config::{MAX_CONFIG_BYTES, MAX_DOCUMENT_BYTES};

use crate::Error;

/// Ceilings on one migrate run, on top of the per-file caps.
///
/// Going over a ceiling stops the run with exit 2 and names the file that crossed it.
/// The file limit counts every directory entry the walk meets, not only the files it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most directory entries one run walks.
    pub max_files: usize,
    /// Most bytes one run reads in total.
    pub max_bytes: u64,
}

impl Limits {
    /// Limits of `max_files` entries and `max_bytes` bytes per run.
    #[must_use]
    pub fn new(max_files: usize, max_bytes: u64) -> Self {
        Self {
            max_files,
            max_bytes,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::new(100_000, 1024 * 1024 * 1024)
    }
}

/// Which per-file cap applies to a read.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Kind {
    Config,
    Document,
}

impl Kind {
    fn cap(self) -> u64 {
        match self {
            Self::Config => MAX_CONFIG_BYTES,
            Self::Document => MAX_DOCUMENT_BYTES,
        }
    }

    fn too_large(self, path: &Path) -> Error {
        let (path, cap) = (path.display().to_string(), self.cap());
        match self {
            Self::Config => Error::ConfigTooLarge { path, cap },
            Self::Document => Error::DocumentTooLarge { path, cap },
        }
    }
}

/// What one run has used so far: walked entries and bytes of text, against its [`Limits`].
#[derive(Debug)]
pub(crate) struct Budget {
    limits: Limits,
    entries: usize,
    bytes: u64,
}

impl Budget {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            entries: 0,
            bytes: 0,
        }
    }

    /// Count one directory entry. Call it before storing the entry.
    pub(crate) fn charge_entry(&mut self, path: &Path) -> Result<(), Error> {
        self.entries = self.entries.saturating_add(1);
        if self.entries > self.limits.max_files {
            return Err(Error::RunLimit {
                path: path.display().to_string(),
                what: "files",
                limit: u64::try_from(self.limits.max_files).unwrap_or(u64::MAX),
            });
        }
        Ok(())
    }

    fn charge_bytes(&mut self, path: &Path, len: usize) -> Result<(), Error> {
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(len).unwrap_or(u64::MAX));
        if self.bytes > self.limits.max_bytes {
            return Err(Error::RunLimit {
                path: path.display().to_string(),
                what: "bytes",
                limit: self.limits.max_bytes,
            });
        }
        Ok(())
    }

    /// Read `path` as UTF-8 text within the cap for `kind` and the run's byte budget.
    pub(crate) fn read(&mut self, path: &Path, kind: Kind) -> Result<String, Error> {
        let file = File::open(path).map_err(|e| Error::io(path, e))?;
        let bytes = slopvac_fs::read_capped(file, kind.cap()).map_err(|e| match e {
            slopvac_fs::Error::CapExceeded { .. } => kind.too_large(path),
            source => Error::read(path, source),
        })?;
        self.charge_bytes(path, bytes.len())?;
        String::from_utf8(bytes).map_err(|_| Error::NotUtf8 {
            path: path.display().to_string(),
        })
    }

    /// Check text that is not read from disk, such as a rewritten document, against the
    /// same cap and budget as a read.
    pub(crate) fn check_text(&mut self, path: &Path, kind: Kind, text: &str) -> Result<(), Error> {
        if u64::try_from(text.len()).map_or(true, |len| len > kind.cap()) {
            return Err(kind.too_large(path));
        }
        self.charge_bytes(path, text.len())
    }
}
