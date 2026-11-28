//! Where a pack's files come from: the binary, or a directory.

use std::path::{Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

use crate::PackPath;
use crate::messages::{PackMessage, code_of};

/// File name of the manifest at the root of a pack.
pub const MANIFEST_FILE: &str = "slopvac-plugin.toml";

/// Largest plugin file the directory source reads, in bytes: the cap of a rule or category file
/// (policies, section 1.9), which is also the largest file a plugin may ship.
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
    /// A plugin file is over the file cap. The code and the text are those of `slopvac-rules`.
    #[error("{}", slopvac_rules::LoadErrorKind::TooLarge(*.cap))]
    TooLarge {
        /// The cap the file went over, in bytes.
        cap: u64,
    },
    /// A style file is over the style cap. The code and the text are those of the styles
    /// catalog, with the file stem as the style name.
    #[error("{}", slopvac_styles::StyleError::TooLarge { style: slopvac_sanitize::line(.style), cap: *.cap })]
    StyleTooLarge {
        /// The file stem of the style.
        style: String,
        /// The cap the file went over, in bytes.
        cap: u64,
    },
    /// A file that must be text is not valid UTF-8. The cause keeps the offset only, never the bytes.
    #[error("{}", PackMessage::FileNotText { path: slopvac_sanitize::line(.path) })]
    NotText {
        /// The pack-relative path of the file.
        path: String,
        /// Where the text stops being UTF-8.
        #[source]
        cause: std::str::Utf8Error,
    },
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
            Self::NotText { .. } => Some(code_of!(FileNotText { path })),
            Self::TooManyEntries => Some(code_of!(TooManyEntries { limit })),
            Self::TooLarge { cap } => slopvac_rules::LoadErrorKind::TooLarge(*cap).code(),
            Self::StyleTooLarge { .. } => Some(slopvac_config::Code::StyleTooLarge.as_str()),
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

    /// Like [`PackSource::read`], and refuses a file over `cap` bytes with
    /// [`SourceError::TooLarge`]. A source that reads a file from disk applies the cap to the
    /// bytes, before they are decoded as UTF-8.
    ///
    /// # Errors
    /// As [`PackSource::read`], and [`SourceError::TooLarge`] for a file over `cap`.
    fn read_capped(&self, path: &PackPath, cap: u64) -> Result<String, SourceError> {
        let text = self.read(path)?;
        if text.len() as u64 > cap {
            return Err(SourceError::TooLarge { cap });
        }
        Ok(text)
    }

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

    fn read_file(&self, rel: &str, cap: u64) -> Result<String, SourceError> {
        // Each read opens the root again and then walks down without following links, so a
        // link that appears between two reads cannot carry a later read outside the pack.
        let root =
            Dir::open_ambient_dir(&self.root, ambient_authority()).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => SourceError::Missing {
                    path: rel.to_owned(),
                },
                _ => SourceError::Read(slopvac_fs::Error::read(&e)),
            })?;
        slopvac_fs::read_capped_regular_to_string_in(&root, Path::new(rel), cap).map_err(
            |e| match e.input_kind() {
                Some(slopvac_fs::InputKind::NotFound) => SourceError::Missing {
                    path: rel.to_owned(),
                },
                Some(slopvac_fs::InputKind::TooLarge) => SourceError::TooLarge { cap },
                _ => SourceError::Read(e),
            },
        )
    }
}

/// A pack held in memory: the files of a tree exactly as one hash pass read them, so a pack
/// loaded from it never opens a file a second time.
#[derive(Debug, Clone)]
pub(crate) struct MemorySource {
    files: std::collections::BTreeMap<String, Vec<u8>>,
}

impl MemorySource {
    /// A source over `files`, keyed by pack-relative path with `/` separators.
    #[must_use]
    pub(crate) fn new(files: std::collections::BTreeMap<String, Vec<u8>>) -> Self {
        Self { files }
    }

    /// The captured bytes of the file at `path`, when the source holds one.
    pub(crate) fn bytes(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    fn text(&self, path: &str, cap: u64) -> Result<String, SourceError> {
        let bytes = self.files.get(path).ok_or_else(|| SourceError::Missing {
            path: path.to_owned(),
        })?;
        if bytes.len() as u64 > cap {
            return Err(SourceError::TooLarge { cap });
        }
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|cause| SourceError::NotText {
                path: path.to_owned(),
                cause,
            })
    }
}

impl PackSource for MemorySource {
    fn manifest_text(&self) -> Result<String, SourceError> {
        self.text(MANIFEST_FILE, MAX_FILE_BYTES)
    }

    fn read(&self, path: &PackPath) -> Result<String, SourceError> {
        self.text(path.as_str(), MAX_FILE_BYTES)
    }

    fn read_capped(&self, path: &PackPath, cap: u64) -> Result<String, SourceError> {
        self.text(path.as_str(), cap.min(MAX_FILE_BYTES))
    }

    fn files(&self) -> Result<Vec<String>, SourceError> {
        Ok(self
            .files
            .keys()
            .filter(|p| p.as_str() != MANIFEST_FILE)
            .cloned()
            .collect())
    }
}

impl PackSource for DirSource {
    fn manifest_text(&self) -> Result<String, SourceError> {
        self.read_file(MANIFEST_FILE, MAX_FILE_BYTES)
    }

    fn read(&self, path: &PackPath) -> Result<String, SourceError> {
        self.read_file(path.as_str(), MAX_FILE_BYTES)
    }

    fn read_capped(&self, path: &PackPath, cap: u64) -> Result<String, SourceError> {
        self.read_file(path.as_str(), cap.min(MAX_FILE_BYTES))
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
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]
mod tests {
    use super::*;

    fn source() -> MemorySource {
        MemorySource::new(
            [
                (MANIFEST_FILE, &b"id = \"x\"\n"[..]),
                ("rules/a.yml", b"rule\n"),
                ("bin/tool", &[0xff, 0xfe]),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_vec()))
            .collect(),
        )
    }

    #[test]
    fn a_memory_source_serves_the_bytes_it_was_given() {
        let s = source();
        let path = PackPath::parse("rules/a.yml").unwrap();
        assert_eq!(s.read(&path).unwrap(), "rule\n");
        assert_eq!(s.manifest_text().unwrap(), "id = \"x\"\n");
    }

    #[test]
    fn a_memory_source_lists_every_file_but_the_manifest() {
        assert_eq!(source().files().unwrap(), ["bin/tool", "rules/a.yml"]);
    }

    #[test]
    fn a_memory_source_reports_a_missing_file_and_one_over_the_cap() {
        let s = source();
        let absent = PackPath::parse("rules/b.yml").unwrap();
        assert!(matches!(s.read(&absent), Err(SourceError::Missing { .. })));
        let present = PackPath::parse("rules/a.yml").unwrap();
        assert!(matches!(
            s.read_capped(&present, 2),
            Err(SourceError::TooLarge { cap: 2 })
        ));
    }

    #[test]
    fn a_memory_source_refuses_a_file_that_is_not_text() {
        let path = PackPath::parse("bin/tool").unwrap();
        assert!(matches!(
            source().read(&path),
            Err(SourceError::NotText { .. })
        ));
    }
}
