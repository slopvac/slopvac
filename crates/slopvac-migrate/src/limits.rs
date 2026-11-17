//! Input caps and per-run limits for everything migrate reads.

use std::fs::File;
use std::path::Path;

use crate::Error;

// TODO(8l4.21.8): replace with slopvac_config re-export from #533
/// Largest config file migrate reads: 1 MiB (`config-too-large`).
pub(crate) const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

// TODO(8l4.21.8): replace with slopvac_config re-export from #533
/// Largest document migrate reads: 64 MiB (`document-too-large`).
pub(crate) const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

/// Ceilings on one migrate run, on top of the per-file caps.
///
/// Going over a ceiling stops the run with exit 2 and names the file that crossed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most files one run reads.
    pub max_files: usize,
    /// Most bytes one run reads in total.
    pub max_bytes: u64,
}

impl Limits {
    /// Limits of `max_files` files and `max_bytes` bytes per run.
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

/// Reads text files within the per-file caps and the per-run [`Limits`].
#[derive(Debug)]
pub(crate) struct Reader {
    limits: Limits,
    files: usize,
    bytes: u64,
}

impl Reader {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            files: 0,
            bytes: 0,
        }
    }

    /// Read `path` as UTF-8 text.
    pub(crate) fn read(&mut self, path: &Path, kind: Kind) -> Result<String, Error> {
        let shown = path.display().to_string();
        self.files = self.files.saturating_add(1);
        if self.files > self.limits.max_files {
            return Err(Error::RunLimit {
                path: shown,
                what: "files",
                limit: u64::try_from(self.limits.max_files).unwrap_or(u64::MAX),
            });
        }
        let cap = match kind {
            Kind::Config => MAX_CONFIG_BYTES,
            Kind::Document => MAX_DOCUMENT_BYTES,
        };
        let file = File::open(path).map_err(|e| Error::io(path, e))?;
        let bytes = slopvac_fs::read_capped(file, cap).map_err(|e| match e {
            slopvac_fs::Error::CapExceeded { cap } => match kind {
                Kind::Config => Error::ConfigTooLarge { path: shown, cap },
                Kind::Document => Error::DocumentTooLarge { path: shown, cap },
            },
            other => Error::io(path, std::io::Error::other(other.to_string())),
        })?;
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        if self.bytes > self.limits.max_bytes {
            return Err(Error::RunLimit {
                path: path.display().to_string(),
                what: "bytes",
                limit: self.limits.max_bytes,
            });
        }
        String::from_utf8(bytes).map_err(|_| Error::NotUtf8 {
            path: path.display().to_string(),
        })
    }
}
