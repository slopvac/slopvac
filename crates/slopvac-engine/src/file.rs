//! Fixing a file on disk: open it safely, lint it, fix it, commit the result atomically.
//!
//! [`fix_file`] never writes through the path it was given. It opens the file without
//! following a symbolic link, reads through that handle, and records the file's identity
//! (device, inode, length, modification time). To write it puts the fixed text in a
//! temporary file in the same directory, copies the mode, flushes it, and then checks
//! again, immediately before the rename, that the path still names the same single-linked
//! file with the same bytes. Only then does it rename the temporary file over the path. A
//! check that fails leaves the file untouched and reports [`Refusal::Changed`].
//!
//! A check followed by a rename cannot be one atomic step with the standard library, so a
//! swap in the instant between them still gets through. The window is a few
//! system calls wide and the worst result is that a file the user could write is replaced
//! by a fixed copy of the text that was linted.
//!
//! Windows has no stable safe call for a file's hard-link count or its file id, so there
//! the checks use the length, the modification time and the bytes, and hard links are not
//! detected.

use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Seek, Write};
use std::ops::RangeInclusive;
use std::path::Path;
use std::time::SystemTime;

use slopvac_model::Document;

use crate::fix::{FixError, FixReport, fix_document, unified_diff};
use crate::lint::{Engine, Input, Kind};

/// Why [`fix_file`] left a file unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// The path is a symbolic link. Writing would change the target.
    Symlink,
    /// The file has more than one hard link. Writing would change every name.
    Hardlinked,
    /// The file changed, or the path was swapped, between the read and the commit.
    Changed,
}

/// What [`fix_file`] did to one file.
#[derive(Debug, Clone)]
pub struct FileFix {
    /// The lint report: of the fixed text when the file was written, else of the file as
    /// it was.
    pub document: Document,
    /// The fix result. Empty and unchanged when the file was refused.
    pub report: FixReport,
    /// The unified diff of the change, empty when nothing changed or the file was refused.
    pub diff: String,
    /// Whether the file was written.
    pub written: bool,
    /// Why the file was left alone, if it was.
    pub refused: Option<Refusal>,
}

/// Lint `path`, fix it and, unless `dry_run`, write the result atomically.
///
/// A symbolic link or a file with several hard links is refused and left unchanged, and so
/// is a file that changes while the fix runs. The lint report of a written file is rerun on
/// the fixed text, so a caller scores what is on disk. A dry run changes nothing and
/// reports the diff the write would make. See the module notes for the write protocol.
pub fn fix_file(
    engine: &Engine,
    path: &Path,
    kind: Kind,
    dry_run: bool,
    changed_lines: Option<&[RangeInclusive<usize>]>,
) -> Result<FileFix, FixError> {
    fix_file_hooked(engine, path, kind, dry_run, changed_lines, &mut || {})
}

/// [`fix_file`] with a callback that runs after the fixed copy is written and before the
/// commit checks. Tests use it to change the file at the worst moment.
#[doc(hidden)]
pub fn fix_file_hooked(
    engine: &Engine,
    path: &Path,
    kind: Kind,
    dry_run: bool,
    changed_lines: Option<&[RangeInclusive<usize>]>,
    before_commit: &mut dyn FnMut(),
) -> Result<FileFix, FixError> {
    let shown = path.to_string_lossy().into_owned();
    let io = |source| FixError::Io {
        path: shown.clone(),
        source,
    };
    let (mut file, before, bytes) = match open_checked(path).map_err(io)? {
        Opened::Refused(refusal) => {
            let bytes = std::fs::read(path).map_err(io)?;
            let source = String::from_utf8(bytes).map_err(|_| FixError::NotUtf8(shown.clone()))?;
            let document = engine.lint(&Input {
                kind,
                ..Input::markdown(&shown, &source)
            })?;
            return Ok(unchanged(document, source, Some(refusal)));
        }
        Opened::File(mut file, meta) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(io)?;
            (file, meta, bytes)
        }
    };
    let source = String::from_utf8(bytes.clone()).map_err(|_| FixError::NotUtf8(shown.clone()))?;
    let input = Input {
        kind,
        ..Input::markdown(&shown, &source)
    };
    let document = engine.lint(&input)?;
    let report = fix_document(&input, &document, changed_lines)?;
    let diff = unified_diff(&shown, &source, &report.text);
    if dry_run || report.text == source {
        return Ok(FileFix {
            document,
            report,
            diff,
            written: false,
            refused: None,
        });
    }
    let temp = write_temp(path, &report.text, &before).map_err(io)?;
    before_commit();
    if !unchanged_since(path, &mut file, &before, &bytes).map_err(io)? {
        return Ok(unchanged(document, source, Some(Refusal::Changed)));
    }
    drop(file);
    temp.persist(path).map_err(|e| io(e.error))?;
    sync_directory(path);
    let document = engine.lint(&Input {
        source: &report.text,
        ..input
    })?;
    Ok(FileFix {
        document,
        report,
        diff,
        written: true,
        refused: None,
    })
}

fn unchanged(document: Document, source: String, refused: Option<Refusal>) -> FileFix {
    FileFix {
        document,
        report: FixReport {
            text: source,
            applied: 0,
            skipped: Vec::new(),
        },
        diff: String::new(),
        written: false,
        refused,
    }
}

enum Opened {
    File(File, Metadata),
    Refused(Refusal),
}

/// Open `path` for reading without following a link, and refuse what must not be written.
fn open_checked(path: &Path) -> std::io::Result<Opened> {
    let named = std::fs::symlink_metadata(path)?;
    if is_link(&named) {
        return Ok(Opened::Refused(Refusal::Symlink));
    }
    if !named.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let file = match open_no_follow(path) {
        Ok(file) => file,
        Err(e) if is_loop(&e) => return Ok(Opened::Refused(Refusal::Symlink)),
        Err(e) => return Err(e),
    };
    let held = file.metadata()?;
    if !held.is_file() || identity(&held) != identity(&named) {
        return Ok(Opened::Refused(Refusal::Changed));
    }
    if linked(&held) {
        return Ok(Opened::Refused(Refusal::Hardlinked));
    }
    Ok(Opened::File(file, held))
}

/// Write the fixed text to a temporary file next to `path` with the mode of `original`.
fn write_temp(
    path: &Path,
    text: &str,
    original: &Metadata,
) -> std::io::Result<tempfile::NamedTempFile> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let mut temp = tempfile::Builder::new()
        .prefix(".slopvac-fix-")
        .tempfile_in(dir)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().set_permissions(original.permissions())?;
    keep_owner(temp.path(), original);
    temp.as_file().sync_all()?;
    Ok(temp)
}

/// Whether `path` still names the file that was read, unchanged and singly linked.
fn unchanged_since(
    path: &Path,
    held: &mut File,
    before: &Metadata,
    bytes: &[u8],
) -> std::io::Result<bool> {
    let named = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if is_link(&named) || !named.is_file() || linked(&named) {
        return Ok(false);
    }
    let open = held.metadata()?;
    if identity(&named) != identity(before) || identity(&open) != identity(before) {
        return Ok(false);
    }
    held.rewind()?;
    let mut now = Vec::with_capacity(bytes.len());
    held.read_to_end(&mut now)?;
    Ok(now == bytes)
}

/// What names a file: where it lives, how long it is and when it changed.
#[derive(Debug, PartialEq, Eq)]
struct Identity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    len: u64,
    modified: Option<SystemTime>,
}

fn identity(meta: &Metadata) -> Identity {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Identity {
        #[cfg(unix)]
        device: meta.dev(),
        #[cfg(unix)]
        inode: meta.ino(),
        len: meta.len(),
        modified: meta.modified().ok(),
    }
}

#[cfg(unix)]
fn linked(meta: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.nlink() > 1
}

#[cfg(not(unix))]
fn linked(_: &Metadata) -> bool {
    false
}

fn is_link(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT: symbolic links and junctions.
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    // NONBLOCK so a FIFO swapped in after the check cannot block the open.
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    let flags = i32::try_from(flags.bits()).map_err(|_| std::io::Error::other("flags"))?;
    OpenOptions::new().read(true).custom_flags(flags).open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

#[cfg(unix)]
fn is_loop(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())
}

#[cfg(not(unix))]
fn is_loop(_: &std::io::Error) -> bool {
    false
}

/// Give the temporary file the owner and group of the original where the system allows it.
#[cfg(unix)]
fn keep_owner(temp: &Path, original: &Metadata) {
    use std::os::unix::fs::{MetadataExt, chown};
    let me = rustix::process::getuid().as_raw();
    let owner = (original.uid() != me).then_some(original.uid());
    // Best effort: only root can give a file away.
    let _ = chown(temp, owner, Some(original.gid()));
}

#[cfg(not(unix))]
fn keep_owner(_: &Path, _: &Metadata) {}

/// Flush the directory entry of the renamed file.
fn sync_directory(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        if let Ok(dir) = File::open(dir) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}
