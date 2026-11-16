//! Open a changed file without following a symlink or blocking on a special file.
//!
//! The scope is built before the lint reads files, so the work tree can change between
//! the two. A file git reported as regular may have become a symlink to something outside
//! the repository, or a FIFO. The open itself refuses both, so there is no gap between a
//! check and the read.
//!
//! - Unix: `O_NOFOLLOW` fails on a symlink as the last component, and `O_NONBLOCK` stops
//!   a FIFO open from waiting for a writer.
//! - Windows: `FILE_FLAG_OPEN_REPARSE_POINT` opens a symlink or junction itself rather
//!   than its target, and the handle's metadata then shows it.
//!
//! Only the last path component is guarded; the directories above it come from
//! [`super::ChangedLines`]'s canonical root.

use std::fs::File;
use std::path::Path;

use super::DiffError;

/// Open `path` for reading.
///
/// `Ok(None)` means the file is absent or cannot be opened, so the caller keeps git's
/// numbers. A symlink, a file that is not regular, or a file over `max_file` bytes is an
/// error.
pub(super) fn open_regular(path: &Path, max_file: u64) -> Result<Option<File>, DiffError> {
    let Some(file) = open_no_follow(path)? else {
        return Ok(None);
    };
    let Ok(meta) = file.metadata() else {
        return Ok(None);
    };
    if meta.file_type().is_symlink() {
        return Err(DiffError::Symlink(path.to_path_buf()));
    }
    if !meta.is_file() {
        return Err(DiffError::NotRegularFile(path.to_path_buf()));
    }
    if meta.len() > max_file {
        return Err(DiffError::FileTooLarge {
            path: path.to_path_buf(),
            limit: max_file,
        });
    }
    Ok(Some(file))
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<Option<File>, DiffError> {
    use rustix::fs::{Mode, OFlags, open};
    use rustix::io::Errno;

    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    match open(path, flags, Mode::empty()) {
        Ok(fd) => Ok(Some(File::from(fd))),
        // ELOOP on Linux and macOS, EMLINK on FreeBSD: the last component is a symlink.
        Err(Errno::LOOP | Errno::MLINK) => Err(DiffError::Symlink(path.to_path_buf())),
        Err(_) => Ok(None),
    }
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> Result<Option<File>, DiffError> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    Ok(std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok())
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow(path: &Path) -> Result<Option<File>, DiffError> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(DiffError::Symlink(path.to_path_buf()));
    }
    Ok(File::open(path).ok())
}
