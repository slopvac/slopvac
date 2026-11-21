//! Fixing a file on disk without trusting the path it was found under.
//!
//! [`fix_file`] takes the project root the caller trusts and a path inside it. It never
//! hands the path to the system whole. The root is opened once, as a [`Root`] directory
//! handle, and never again by name, so replacing the root path with a link later changes
//! nothing. It walks to the file one component at a time relative to that handle, never following a link, so a
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
//! A file with an access ACL, or in a directory whose ACL new files would inherit, is
//! refused with [`Refusal::AclPresent`]. Linux reads the ACL attributes. macOS hides its
//! ACL from the attribute calls and reading it takes an `unsafe` foreign call, so it asks
//! `ls -lde` for the entries instead. A failure of that check fails the write.
//!
//! In-place writes exist on unix only. Elsewhere a dry run and a diff work, and a write is
//! refused with [`Refusal::UnsupportedPlatform`] (bead slopvac-8l4.3.35 tracks a safe
//! implementation).

use std::ops::RangeInclusive;
use std::path::{Component, Path, PathBuf};

use slopvac_model::Document;

use crate::commit::{Refusal, Root, Step, open};
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

/// How the caller linted a file, so the fix run lints it the same way.
///
/// [`fix_file`] knows only the path and the kind. A caller that gave its lint run a display
/// path, a root-relative path for `[[genres]]` matching or a declared genre sets the same
/// here, or the fix could act on findings the lint run did not report.
#[derive(Debug, Clone, Copy)]
pub struct FileContext<'a> {
    /// The kind of file.
    pub kind: Kind,
    /// The path to show in the report and in errors. Defaults to `path`.
    pub shown: Option<&'a str>,
    /// The path relative to the project root, for matching `[[genres]]` file patterns.
    pub relative_path: Option<&'a str>,
    /// The genre the document declares, as the config reader validated it.
    pub declared_genre: Option<&'a str>,
}

impl FileContext<'_> {
    /// A context that knows only the kind of file.
    #[must_use]
    pub const fn new(kind: Kind) -> Self {
        Self {
            kind,
            shown: None,
            relative_path: None,
            declared_genre: None,
        }
    }
}

/// Lint the file at `path`, fix it and, unless `dry_run`, write the result.
///
/// `root` is the project root the caller trusts, opened here once; `path` is absolute inside it or relative
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
    let root = Root::open(root).map_err(|source| FixError::Io {
        path: path.to_string_lossy().into_owned(),
        source,
    })?;
    fix_file_hooked(
        engine,
        &root,
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
    root: &Root,
    path: &Path,
    kind: Kind,
    dry_run: bool,
    changed_lines: Option<&[RangeInclusive<usize>]>,
    hook: &mut dyn FnMut(Step) -> std::io::Result<()>,
) -> Result<FileFix, FixError> {
    fix_file_in(
        engine,
        root,
        path,
        &FileContext::new(kind),
        dry_run,
        changed_lines,
        hook,
    )
}

/// [`fix_file_hooked`] with the lint context of the caller (see [`FileContext`]).
#[doc(hidden)]
pub fn fix_file_in(
    engine: &Engine,
    root: &Root,
    path: &Path,
    context: &FileContext<'_>,
    dry_run: bool,
    changed_lines: Option<&[RangeInclusive<usize>]>,
    hook: &mut dyn FnMut(Step) -> std::io::Result<()>,
) -> Result<FileFix, FixError> {
    let kind = context.kind;
    let shown = context
        .shown
        .map_or_else(|| path.to_string_lossy().into_owned(), str::to_owned);
    let io = |source| FixError::Io {
        path: shown.clone(),
        source,
    };
    let Some(relative) = relative_to(root.path(), path) else {
        return Ok(FileFix::Refused(Refusal::OutsideRoot));
    };
    let (mut handle, bytes) = match open(root, &relative).map_err(io)? {
        Ok(opened) => opened,
        Err(refusal) => return Ok(FileFix::Refused(refusal)),
    };
    let source = String::from_utf8(bytes).map_err(|_| FixError::NotUtf8(shown.clone()))?;
    let mut input = Input::new(&shown, &source).with_kind(kind);
    if let Some(relative) = context.relative_path {
        input = input.with_relative_path(relative);
    }
    if let Some(genre) = context.declared_genre {
        input = input.with_declared_genre(genre);
    }
    let document = engine.lint(&input)?;
    let report = fix_document(engine, &input, &document, changed_lines)?;
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
    let document = engine.lint(&input.with_source(&report.text))?;
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
        path.strip_prefix(root).ok().map(Path::to_path_buf)?
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
