//! Where a pack's files come from: the binary, or a directory.

use std::path::PathBuf;

use crate::PackPath;

/// File name of the manifest at the root of a pack.
pub const MANIFEST_FILE: &str = "slopvac-pack.toml";

/// Largest pack file the directory source reads, in bytes.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// Why a pack file could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SourceError {
    /// The pack has no such file.
    #[error("the pack has no file `{}`", slopvac_sanitize::line(.path))]
    Missing {
        /// The pack-relative path.
        path: String,
    },
    /// The file exists but could not be read.
    #[error("cannot read a pack file")]
    Read(#[from] slopvac_fs::Error),
}

/// A pack's files, read by pack-relative path.
pub trait PackSource {
    /// The text of the manifest ([`MANIFEST_FILE`]).
    ///
    /// # Errors
    /// [`SourceError`] when the manifest is absent or unreadable.
    fn manifest_text(&self) -> Result<String, SourceError>;

    /// The text of the file at `path`.
    ///
    /// # Errors
    /// [`SourceError::Missing`] when the pack has no such file.
    fn read(&self, path: &PackPath) -> Result<String, SourceError>;
}

/// One embedded table of `(path, text)` files.
pub type FileTable = &'static [(&'static str, &'static str)];

/// A pack compiled into the binary: a manifest and tables of `(path, text)` files.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedSource {
    manifest: &'static str,
    tables: &'static [FileTable],
}

impl EmbeddedSource {
    /// A source over `manifest` and the files in `tables`.
    #[must_use]
    pub const fn new(manifest: &'static str, tables: &'static [FileTable]) -> Self {
        Self { manifest, tables }
    }

    /// Every embedded file as `(path, text)`, manifest excluded.
    pub fn files(&self) -> impl Iterator<Item = (&'static str, &'static str)> {
        self.tables.iter().flat_map(|t| t.iter().copied())
    }
}

impl PackSource for EmbeddedSource {
    fn manifest_text(&self) -> Result<String, SourceError> {
        Ok(self.manifest.to_owned())
    }

    fn read(&self, path: &PackPath) -> Result<String, SourceError> {
        self.files()
            .find(|(p, _)| *p == path.as_str())
            .map(|(_, text)| text.to_owned())
            .ok_or_else(|| SourceError::Missing {
                path: path.as_str().to_owned(),
            })
    }
}

/// A pack in a directory, with its manifest at the root. A file that is a symlink is refused.
#[derive(Debug, Clone)]
pub struct DirSource {
    root: PathBuf,
}

impl DirSource {
    /// A source rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn read_file(&self, rel: &str) -> Result<String, SourceError> {
        let path = self.root.join(rel);
        slopvac_fs::read_capped_regular_to_string(
            &path,
            MAX_FILE_BYTES,
            slopvac_fs::ReadOptions::new(),
        )
        .map_err(|e| match e.input_kind() {
            Some(slopvac_fs::InputKind::NotFound) => SourceError::Missing {
                path: rel.to_owned(),
            },
            _ => SourceError::Read(e),
        })
    }
}

impl PackSource for DirSource {
    fn manifest_text(&self) -> Result<String, SourceError> {
        self.read_file(MANIFEST_FILE)
    }

    fn read(&self, path: &PackPath) -> Result<String, SourceError> {
        self.read_file(path.as_str())
    }
}
