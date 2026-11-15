//! Fixing a file on disk without trusting the path it was found under.
//!
//! [`fix_file`] takes the project root the caller trusts and a path inside it. It never
//! hands the path to the system whole. It opens the root as a directory handle and walks to
//! the file one component at a time relative to that handle, never following a link, so a
//! symbolic link or other reparse point in any component ends the walk with
//! [`Refusal::Symlink`]. It checks the open file (regular, one link, size, and on unix an
//! owner that is the current user) and reads through that handle only. Nothing that is
//! refused is read. The platform layer is `commit.rs`, built on `cap-std`, the crate the
//! plugin host also uses.
//!
//! To write, it stages the fixed text in a temporary file created with `O_EXCL` semantics in
//! the same directory, gives it the original's mode and group (a group it cannot give is
//! refused rather than widened), flushes it, and checks again that the file is the one it
//! read and that the temporary name still holds the staged file. It then renames over the
//! target within the directory handle. It never changes the owner. Any failure removes the
//! temporary file and leaves the original as it was. On unix the parent directory must also
//! refuse writes from group and others, with a sticky directory owned by the current user
//! as the exception.
//!
//! The re-check and the rename cannot be one system call, so a change inside that gap is
//! not caught. Only the directory's owner or root can make one, because a write needs a
//! directory that group and others cannot change.
//!
//! A file with an access ACL, or in a directory with a default ACL, is refused with
//! [`Refusal::AclPresent`] on Linux. macOS keeps ACLs in an attribute that `rustix` cannot
//! read, so an ACL is not detected there.
//!
//! In-place writes exist on unix only. Elsewhere a dry run and a diff work, and a write is
//! refused with [`Refusal::UnsupportedPlatform`] (bead slopvac-8l4.3.35 tracks a safe
//! implementation).

use std::ops::RangeInclusive;
use std::path::{Component, Path, PathBuf};

use slopvac_model::Document;

use crate::commit::{Refusal, Step, open};
use crate::fix::{FixError, FixReport, fix_document, unified_diff};
use crate::lint::{Engine, Input, Kind};

/// What [`fix_file`] did.
#[derive(Debug, Clone)]
pub enum FileFix {
    /// The file was not read or not changed, and why.
    Refused(Refusal),
    /// The file was linted and fixed.
    Done(Box<FileResult>),
}

/// The outcome for a file that was not refused.
#[derive(Debug, Clone)]
pub struct FileResult {
    /// The lint report: of the fixed text when the file was written, else of the file as
    /// it was.
    pub document: Document,
    /// The fix result.
    pub report: FixReport,
    /// The unified diff of the change, empty when nothing changed.
    pub diff: String,
    /// Whether the file was written.
    pub written: bool,
}

/// Lint the file at `path`, fix it and, unless `dry_run`, write the result.
///
/// `root` is the project root the caller trusts; `path` is absolute inside it or relative
/// to it, with no `..`. See the module notes for what is refused and how the write is
/// made. The lint report of a written file is rerun on the fixed text. A dry run changes
/// nothing and reports the diff the write would make.
pub fn fix_file(
    engine: &Engine,
    root: &Path,
    path: &Path,
    kind: Kind,
    dry_run: bool,
    changed_lines: Option<&[RangeInclusive<usize>]>,
) -> Result<FileFix, FixError> {
    fix_file_hooked(
        engine,
        root,
        path,
        kind,
        dry_run,
        changed_lines,
        &mut |_| Ok(()),
    )
}

/// [`fix_file`] with a callback that runs before each [`Step`] of the write. An error from
/// it fails that step. Tests use it to inject failures and to change files at the worst
/// moment.
#[doc(hidden)]
pub fn fix_file_hooked(
    engine: &Engine,
    root: &Path,
    path: &Path,
    kind: Kind,
    dry_run: bool,
    changed_lines: Option<&[RangeInclusive<usize>]>,
    hook: &mut dyn FnMut(Step) -> std::io::Result<()>,
) -> Result<FileFix, FixError> {
    let shown = path.to_string_lossy().into_owned();
    let io = |source| FixError::Io {
        path: shown.clone(),
        source,
    };
    let Some(relative) = relative_to(root, path) else {
        return Ok(FileFix::Refused(Refusal::OutsideRoot));
    };
    let (mut handle, bytes) = match open(root, &relative).map_err(io)? {
        Ok(opened) => opened,
        Err(refusal) => return Ok(FileFix::Refused(refusal)),
    };
    let source = String::from_utf8(bytes).map_err(|_| FixError::NotUtf8(shown.clone()))?;
    let input = Input {
        kind,
        ..Input::markdown(&shown, &source)
    };
    let document = engine.lint(&input)?;
    let report = fix_document(&input, &document, changed_lines)?;
    let diff = unified_diff(&shown, &source, &report.text);
    if dry_run || report.text == source {
        return Ok(FileFix::Done(Box::new(FileResult {
            document,
            report,
            diff,
            written: false,
        })));
    }
    if let Some(refusal) = handle
        .commit(&report.text, source.as_bytes(), hook)
        .map_err(io)?
    {
        return Ok(FileFix::Refused(refusal));
    }
    let document = engine.lint(&Input {
        source: &report.text,
        ..input
    })?;
    Ok(FileFix::Done(Box::new(FileResult {
        document,
        report,
        diff,
        written: true,
    })))
}

/// `path` below `root` as plain names, or `None` when it is outside or climbs with `..`.
fn relative_to(root: &Path, path: &Path) -> Option<PathBuf> {
    let relative = if path.is_absolute() {
        path.strip_prefix(root)
            .ok()
            .map(Path::to_path_buf)
            .or_else(|| {
                let canonical = root.canonicalize().ok()?;
                path.strip_prefix(canonical).ok().map(Path::to_path_buf)
            })?
    } else {
        path.to_path_buf()
    };
    let mut clean = PathBuf::new();
    for part in relative.components() {
        match part {
            Component::Normal(name) => clean.push(name),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (clean.components().count() > 0).then_some(clean)
}
