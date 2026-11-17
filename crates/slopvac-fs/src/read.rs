//! Reads of regular files within a cap, confined and without following links.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::Path;

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, File, MetadataExt, OpenOptions, OpenOptionsExt};
use rustix::fs::OFlags;

use crate::io::split_plain;
use crate::{Error, InputKind, Reason, Result, read_capped};

/// Options for [`read_capped_regular`] and [`read_capped_regular_to_string`].
///
/// Build one with [`ReadOptions::new`] and the chained setters.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct ReadOptions {
    /// Refuse a file that has more than one hard link, with [`InputKind::MultiplyLinked`].
    /// Off by default here; the confined `_in` readers always refuse.
    pub reject_multiply_linked: bool,
}

impl ReadOptions {
    /// The defaults: hard-linked files are accepted.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            reject_multiply_linked: false,
        }
    }

    /// Refuse a file with more than one hard link, or accept it.
    #[must_use]
    pub const fn reject_multiply_linked(mut self, reject: bool) -> Self {
        self.reject_multiply_linked = reject;
        self
    }
}

/// Read the regular file at `rel` below `root`, failing when it holds more than `cap` bytes.
///
/// - `rel` may have several components. Each one is opened through a directory handle without
///   following links, so a symlink anywhere on the way, the last component included, fails
///   with [`InputKind::Symlink`]. `..` and a root are refused with [`Error::UnsafeTarget`].
///   (`root` itself is whatever the caller opened.)
/// - The file opens non-blocking, so a FIFO with no writer cannot hang the call. Anything but
///   a regular file is refused with [`InputKind::NotRegular`]. Then the non-blocking flag is
///   cleared and the read is capped, so memory stays bounded by `cap + 1` bytes.
/// - A file with more than one hard link is refused with [`InputKind::MultiplyLinked`].
/// - Errors carry `rel` as [`Error::path`] and never put it in their text. A missing file is
///   [`InputKind::NotFound`], apart from other failures, so a caller can treat it as normal.
///
/// Like [`write_atomic`](crate::write_atomic), this assumes no other process can change the
/// directories involved while the call runs.
pub fn read_capped_regular_in(root: &Dir, rel: &Path, cap: u64) -> Result<Vec<u8>> {
    let (name, ancestors) = split_plain(rel)?;
    read_in(root, &ancestors, name, rel, cap, true)
}

/// [`read_capped_regular_in`] returning the content as UTF-8, or [`InputKind::NotUtf8`].
pub fn read_capped_regular_to_string_in(root: &Dir, rel: &Path, cap: u64) -> Result<String> {
    into_string(read_capped_regular_in(root, rel, cap)?, rel)
}

/// Read the regular file at `path`, failing when it holds more than `cap` bytes.
///
/// This opens the directory that holds the file the way `std` does, following symlinks in the
/// directories above the file. The file itself must not be a symlink. The checks on the file
/// are those of [`read_capped_regular_in`], except that a hard-linked file is accepted unless
/// [`ReadOptions::reject_multiply_linked`] is set.
pub fn read_capped_regular(path: &Path, cap: u64, opts: ReadOptions) -> Result<Vec<u8>> {
    let Some(name) = path.file_name() else {
        return Err(Error::unsafe_target(Reason::NoFileName, path));
    };
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let dir =
        Dir::open_ambient_dir(dir, ambient_authority()).map_err(|e| classify_open(&e, path))?;
    read_in(&dir, &[], name, path, cap, opts.reject_multiply_linked)
}

/// [`read_capped_regular`] returning the content as UTF-8, or [`InputKind::NotUtf8`].
pub fn read_capped_regular_to_string(path: &Path, cap: u64, opts: ReadOptions) -> Result<String> {
    into_string(read_capped_regular(path, cap, opts)?, path)
}

fn into_string(bytes: Vec<u8>, report: &Path) -> Result<String> {
    String::from_utf8(bytes).map_err(|_| Error::input(InputKind::NotUtf8, report, None))
}

fn read_in(
    root: &Dir,
    ancestors: &[&OsStr],
    name: &OsStr,
    report: &Path,
    cap: u64,
    reject_links: bool,
) -> Result<Vec<u8>> {
    let mut dir = root
        .try_clone()
        .map_err(|e| Error::input(InputKind::Io, report, Some(&e)))?;
    for ancestor in ancestors {
        dir = match dir.open_dir_nofollow(ancestor) {
            Ok(d) => d,
            Err(e) => return Err(classify_step(&dir, ancestor, &e, report)),
        };
    }
    let file = match open_nonblocking(&dir, name) {
        Ok(f) => f,
        Err(e) => return Err(classify_step(&dir, name, &e, report)),
    };
    let meta = file
        .metadata()
        .map_err(|e| Error::input(InputKind::Io, report, Some(&e)))?;
    if !meta.is_file() {
        return Err(Error::input(InputKind::NotRegular, report, None));
    }
    if reject_links && meta.nlink() > 1 {
        return Err(Error::input(InputKind::MultiplyLinked, report, None));
    }
    if meta.len() > cap {
        return Err(Error::input(InputKind::TooLarge, report, None));
    }
    clear_nonblocking(&file).map_err(|e| Error::input(InputKind::Io, report, Some(&e)))?;
    read_capped(file, cap).map_err(|e| match e {
        Error::CapExceeded { .. } => Error::input(InputKind::TooLarge, report, None),
        Error::Read(source) => Error::input(InputKind::Io, report, Some(&source)),
        other => other,
    })
}

fn open_nonblocking(dir: &Dir, name: &OsStr) -> std::io::Result<File> {
    let flags = i32::try_from(OFlags::NONBLOCK.bits()).map_err(std::io::Error::other)?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(FollowSymlinks::No)
        .custom_flags(flags);
    dir.open_with(name, &options)
}

fn clear_nonblocking(file: &File) -> std::io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(file)?;
    rustix::fs::fcntl_setfl(file, flags - OFlags::NONBLOCK)?;
    Ok(())
}

/// Name why opening `name` in `dir` failed: a symlink there, nothing there, or something else.
fn classify_step(dir: &Dir, name: &OsStr, error: &std::io::Error, report: &Path) -> Error {
    match dir.symlink_metadata(name) {
        Ok(meta) if meta.is_symlink() => Error::input(InputKind::Symlink, report, None),
        // Something that is not a directory sits on the way: nothing can be below it.
        Ok(meta) if !meta.is_dir() && error.kind() == ErrorKind::NotADirectory => {
            Error::input(InputKind::NotFound, report, None)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            Error::input(InputKind::NotFound, report, None)
        }
        _ => classify_open(error, report),
    }
}

fn classify_open(error: &std::io::Error, report: &Path) -> Error {
    if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) {
        Error::input(InputKind::NotFound, report, None)
    } else {
        Error::input(InputKind::Io, report, Some(error))
    }
}
