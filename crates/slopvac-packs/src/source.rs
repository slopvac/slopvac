//! Where a pack's files come from: the binary, or a directory.

use std::path::{Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

use crate::PackPath;
use crate::messages::{PackMessage, code_of};

/// File name of the manifest at the root of a pack.
pub const MANIFEST_FILE: &str = "slopvac-plugin.toml";

/// Largest pack file the directory source reads, in bytes: the cap of a rule or category file
/// (policies, section 1.9), which is also the largest file a pack may ship.
const MAX_FILE_BYTES: u64 = slopvac_rules::MAX_RULE_FILE_BYTES;

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
    /// A pack file is over the file cap. The code and the text are those of `slopvac-rules`.
    #[error("{}", slopvac_rules::LoadErrorKind::TooLarge(MAX_FILE_BYTES))]
    TooLarge,
    /// The pack holds a link.
    #[error("{}", PackMessage::FileLink { path: slopvac_sanitize::line(.path) })]
    Link {
        /// The pack-relative path of the link.
        path: String,
    },
    /// The pack holds more entries or levels than a pack may.
    #[error("{}", PackMessage::TooManyEntries { limit: MAX_ENTRIES })]
    TooManyEntries,
}

/// Most directory entries the directory source lists, and most levels it descends.
const MAX_ENTRIES: usize = 1024;
const MAX_DEPTH: usize = 16;

impl SourceError {
    /// The stable code of this error, for the variants that have one.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::Link { .. } => Some(code_of!(FileLink { path })),
            Self::TooManyEntries => Some(code_of!(TooManyEntries { limit })),
            Self::TooLarge => slopvac_rules::LoadErrorKind::TooLarge(MAX_FILE_BYTES).code(),
            _ => None,
        }
    }
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

    /// Every file of the pack except the manifest, as pack-relative paths with `/`
    /// separators, sorted. The loader matches a glob in `provides` against this list.
    ///
    /// # Errors
    /// [`SourceError`] when the pack cannot be listed, holds a link or holds too many entries.
    fn files(&self) -> Result<Vec<String>, SourceError>;
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

    fn files(&self) -> Result<Vec<String>, SourceError> {
        let mut out: Vec<String> = Self::files(self).map(|(p, _)| p.to_owned()).collect();
        out.sort();
        Ok(out)
    }

    fn read(&self, path: &PackPath) -> Result<String, SourceError> {
        Self::files(self)
            .find(|(p, _)| *p == path.as_str())
            .map(|(_, text)| text.to_owned())
            .ok_or_else(|| SourceError::Missing {
                path: path.as_str().to_owned(),
            })
    }
}

/// A pack in a directory, with its manifest at the root. A symlink anywhere below the root,
/// in a directory or as the file, is refused. The root itself is whatever the caller names.
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
        // Each read opens the root again and then walks down without following links, so a
        // link that appears between two reads cannot carry a later read outside the pack.
        let root =
            Dir::open_ambient_dir(&self.root, ambient_authority()).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => SourceError::Missing {
                    path: rel.to_owned(),
                },
                _ => SourceError::Read(slopvac_fs::Error::read(&e)),
            })?;
        slopvac_fs::read_capped_regular_to_string_in(&root, Path::new(rel), MAX_FILE_BYTES).map_err(
            |e| match e.input_kind() {
                Some(slopvac_fs::InputKind::NotFound) => SourceError::Missing {
                    path: rel.to_owned(),
                },
                Some(slopvac_fs::InputKind::TooLarge) => SourceError::TooLarge,
                _ => SourceError::Read(e),
            },
        )
    }
}

impl PackSource for DirSource {
    fn manifest_text(&self) -> Result<String, SourceError> {
        self.read_file(MANIFEST_FILE)
    }

    fn read(&self, path: &PackPath) -> Result<String, SourceError> {
        self.read_file(path.as_str())
    }

    fn files(&self) -> Result<Vec<String>, SourceError> {
        let root = Dir::open_ambient_dir(&self.root, ambient_authority())
            .map_err(|e| SourceError::Read(slopvac_fs::Error::read(&e)))?;
        let mut out = Vec::new();
        let mut seen = 0;
        walk(&root, "", 0, &mut seen, &mut out)?;
        out.retain(|p| p != MANIFEST_FILE);
        out.sort();
        Ok(out)
    }
}

/// Collect the files below `dir` without following links. A link, or more than
/// [`MAX_ENTRIES`] entries or [`MAX_DEPTH`] levels, is an error.
fn walk(
    dir: &Dir,
    prefix: &str,
    depth: usize,
    seen: &mut usize,
    out: &mut Vec<String>,
) -> Result<(), SourceError> {
    let read = |e: &std::io::Error| SourceError::Read(slopvac_fs::Error::read(e));
    for entry in dir.entries().map_err(|e| read(&e))? {
        let entry = entry.map_err(|e| read(&e))?;
        *seen += 1;
        if *seen > MAX_ENTRIES || depth > MAX_DEPTH {
            return Err(SourceError::TooManyEntries);
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let kind = entry.file_type().map_err(|e| read(&e))?;
        if kind.is_symlink() {
            return Err(SourceError::Link { path });
        }
        if kind.is_dir() {
            let child = entry.open_dir().map_err(|e| read(&e))?;
            walk(&child, &path, depth + 1, seen, out)?;
        } else if kind.is_file() {
            out.push(path);
        }
    }
    Ok(())
}
