//! Fixing a file on disk without trusting the path it was found under.
//!
//! [`fix_file`] takes the project root the caller trusts and a path inside it. It never
//! hands the path to the system whole. On unix it opens the root directory and walks to the
//! file one component at a time with `openat` and `O_NOFOLLOW`, so a symbolic link in any
//! component ends the walk with [`Refusal::Symlink`]. It opens the file the same way, checks
//! with `fstat` on the open descriptor that it is a regular file with one link that the
//! current user owns, and reads through that descriptor only. Nothing that is refused is
//! read.
//!
//! To write, it requires that the parent directory is not writable by group or others
//! (a sticky directory owned by the current user passes), creates a temporary file in that
//! directory with `O_EXCL`, writes and flushes it, sets the original mode with `fchmod` on
//! the descriptor, and checks again that the file is the one it read and that the temporary
//! name still holds the temporary file. It renames over the target with `renameat` in the
//! same directory and flushes the directory. It never changes ownership. Any failure removes
//! the temporary file and leaves the original as it was.
//!
//! The re-check and the rename cannot be one system call, so a change inside that gap is
//! not caught. With the directory restricted to its owner, only the owner or root can make
//! one.
//!
//! On other platforms the walk canonicalises the parent, requires it inside the canonical
//! root, and rejects any component that is a reparse point or symbolic link. Windows has no
//! stable safe call for a file id or a hard-link count, so there the checks use the length,
//! the modification time and the bytes, and hard links are not detected.

use std::ops::RangeInclusive;
use std::path::{Component, Path, PathBuf};

use slopvac_model::Document;

use crate::fix::{FixError, FixReport, fix_document, unified_diff};
use crate::lint::{Engine, Input, Kind};

/// The largest file [`fix_file`] reads.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Why [`fix_file`] left a file alone. A refused file is never read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// The path is outside the project root.
    OutsideRoot,
    /// The file or a directory above it is a symbolic link.
    Symlink,
    /// The file is not a regular file: a FIFO, a device or a directory.
    NotRegular,
    /// The file has more than one hard link. Writing would change every name.
    Hardlinked,
    /// Another user owns the file.
    NotOwner,
    /// The file is larger than [`MAX_FILE_BYTES`].
    TooLarge,
    /// The parent directory is writable by group or others, so a name in it could be
    /// swapped under the write.
    UnsafeDirectory,
    /// The file changed, or its name was swapped, between the read and the commit.
    Changed,
}

/// Where in a write a [`fix_file_hooked`] callback runs, before the step it names.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Creating the temporary file.
    Create,
    /// Writing the fixed text to it.
    Write,
    /// Setting its mode and flushing it.
    Flush,
    /// Re-checking the original. Tests change the file here.
    Verify,
    /// Renaming over the original. Tests change the temporary name here.
    Rename,
}

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
    let (mut handle, bytes) = match sys::open(root, &relative).map_err(io)? {
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

/// Whether a directory with this mode may hold a file we replace: nobody but the owner can
/// write to it, or it is sticky and ours.
#[cfg(unix)]
fn directory_trusted(mode: u32, owner: u32, effective_user: u32) -> bool {
    mode & 0o022 == 0 || (mode & 0o1000 != 0 && owner == effective_user)
}

/// Whether the current user may fix a file this user owns. Root does not fix other users'
/// files.
#[cfg(unix)]
fn owner_trusted(owner: u32, effective_user: u32) -> bool {
    owner == effective_user
}

#[cfg(unix)]
mod sys {
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::hash::{BuildHasher, Hasher};
    use std::io::{Read, Seek, Write};
    use std::os::fd::OwnedFd;
    use std::path::Path;

    use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat};
    use rustix::io::Errno;

    use super::{MAX_FILE_BYTES, Refusal, Step, directory_trusted, owner_trusted};

    type Hook<'a> = &'a mut dyn FnMut(Step) -> std::io::Result<()>;

    /// What names a file: device, inode, size and modification time, and its link count.
    #[derive(Debug, PartialEq, Eq)]
    struct Identity {
        device: i128,
        inode: i128,
        size: i128,
        modified: (i128, i128),
    }

    fn identity(st: &Stat) -> Identity {
        Identity {
            device: i128::from(st.st_dev),
            inode: i128::from(st.st_ino),
            size: i128::from(st.st_size),
            modified: (i128::from(st.st_mtime), i128::from(st.st_mtime_nsec)),
        }
    }

    fn is_regular(st: &Stat) -> bool {
        FileType::from_raw_mode(st.st_mode) == FileType::RegularFile
    }

    fn links(st: &Stat) -> i128 {
        i128::from(st.st_nlink)
    }

    /// An open file with the directory it was found in.
    pub(super) struct Handle {
        parent: OwnedFd,
        name: OsString,
        file: File,
        before: Identity,
        mode: Mode,
    }

    const DIR_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    /// A component that is a link fails `O_NOFOLLOW` with `ELOOP`, or with `ENOTDIR` when
    /// `O_DIRECTORY` is also set.
    fn refusal_for(error: Errno) -> Result<Refusal, std::io::Error> {
        if error == Errno::LOOP || error == Errno::NOTDIR {
            Ok(Refusal::Symlink)
        } else {
            Err(error.into())
        }
    }

    pub(super) fn open(
        root: &Path,
        relative: &Path,
    ) -> std::io::Result<Result<(Handle, Vec<u8>), Refusal>> {
        let mut parent = rustix::fs::open(
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let mut names: Vec<&OsStr> = relative.iter().collect();
        let Some(name) = names.pop() else {
            return Ok(Err(Refusal::OutsideRoot));
        };
        for directory in names {
            match rustix::fs::openat(&parent, directory, DIR_FLAGS, Mode::empty()) {
                Ok(next) => parent = next,
                Err(e) => return refusal_for(e).map(Err),
            }
        }
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        let fd = match rustix::fs::openat(&parent, name, flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(e) => return refusal_for(e).map(Err),
        };
        let st = rustix::fs::fstat(&fd)?;
        let me = rustix::process::geteuid().as_raw();
        if !is_regular(&st) {
            return Ok(Err(Refusal::NotRegular));
        }
        if links(&st) > 1 {
            return Ok(Err(Refusal::Hardlinked));
        }
        if !owner_trusted(st.st_uid, me) {
            return Ok(Err(Refusal::NotOwner));
        }
        if u64::try_from(st.st_size).map_or(true, |size| size > MAX_FILE_BYTES) {
            return Ok(Err(Refusal::TooLarge));
        }
        let mut file = File::from(fd);
        let bytes = read_all(&mut file)?;
        Ok(Ok((
            Handle {
                parent,
                name: name.to_owned(),
                file,
                before: identity(&st),
                mode: Mode::from_raw_mode(st.st_mode & 0o7777),
            },
            bytes,
        )))
    }

    fn read_all(file: &mut File) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        Read::take(file, MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// A temporary entry that is unlinked on drop unless it was handed over.
    struct Temp {
        parent: OwnedFd,
        name: OsString,
        armed: bool,
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            if self.armed {
                let _ = rustix::fs::unlinkat(&self.parent, self.name.as_os_str(), AtFlags::empty());
            }
        }
    }

    fn temp_name(name: &OsStr) -> OsString {
        let random = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        let mut out = OsString::from(".");
        if name.len() <= 200 {
            out.push(name);
        }
        out.push(format!(".slopvac-{random:016x}"));
        out
    }

    impl Handle {
        /// Replace the file with `text`, or say why not. `original` is what was read.
        pub(super) fn commit(
            &mut self,
            text: &str,
            original: &[u8],
            hook: Hook<'_>,
        ) -> std::io::Result<Option<Refusal>> {
            let me = rustix::process::geteuid().as_raw();
            let dir = rustix::fs::fstat(&self.parent)?;
            if !directory_trusted(u32::from(dir.st_mode) & 0o7777, dir.st_uid, me) {
                return Ok(Some(Refusal::UnsafeDirectory));
            }
            hook(Step::Create)?;
            let (mut temp_file, mut temp) = self.create_temp()?;
            hook(Step::Write)?;
            temp_file.write_all(text.as_bytes())?;
            hook(Step::Flush)?;
            rustix::fs::fchmod(&temp_file, self.mode)?;
            temp_file.sync_all()?;
            hook(Step::Verify)?;
            if !self.unchanged(original)? {
                return Ok(Some(Refusal::Changed));
            }
            hook(Step::Rename)?;
            let ours = rustix::fs::fstat(&temp_file)?;
            let named = rustix::fs::statat(
                &self.parent,
                temp.name.as_os_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            );
            if !named.is_ok_and(|n| n.st_dev == ours.st_dev && n.st_ino == ours.st_ino) {
                // Someone put something else under our name: leave it, and write nothing.
                temp.armed = false;
                return Ok(Some(Refusal::Changed));
            }
            rustix::fs::renameat(
                &self.parent,
                temp.name.as_os_str(),
                &self.parent,
                self.name.as_os_str(),
            )?;
            temp.armed = false;
            let _ = rustix::fs::fsync(&self.parent);
            Ok(None)
        }

        fn create_temp(&self) -> std::io::Result<(File, Temp)> {
            let flags =
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            for _ in 0..16 {
                let name = temp_name(&self.name);
                match rustix::fs::openat(
                    &self.parent,
                    name.as_os_str(),
                    flags,
                    Mode::RUSR | Mode::WUSR,
                ) {
                    Ok(fd) => {
                        let temp = Temp {
                            parent: self.parent.try_clone()?,
                            name,
                            armed: true,
                        };
                        return Ok((File::from(fd), temp));
                    }
                    Err(Errno::EXIST) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Err(std::io::Error::other("no free temporary name"))
        }

        /// Whether the name still leads to the file we read, unchanged and singly linked.
        fn unchanged(&mut self, original: &[u8]) -> std::io::Result<bool> {
            let held = rustix::fs::fstat(&self.file)?;
            if identity(&held) != self.before || links(&held) > 1 {
                return Ok(false);
            }
            let named = match rustix::fs::statat(
                &self.parent,
                self.name.as_os_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(named) => named,
                Err(Errno::NOENT) => return Ok(false),
                Err(e) => return Err(e.into()),
            };
            if !is_regular(&named) || identity(&named) != self.before {
                return Ok(false);
            }
            self.file.rewind()?;
            Ok(read_all(&mut self.file)? == original)
        }
    }
}

#[cfg(not(unix))]
mod sys {
    use std::fs::{File, Metadata, OpenOptions, Permissions};
    use std::io::{Read, Seek, Write};
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    use super::{MAX_FILE_BYTES, Refusal, Step};

    type Hook<'a> = &'a mut dyn FnMut(Step) -> std::io::Result<()>;

    #[derive(Debug, PartialEq, Eq)]
    struct Identity {
        len: u64,
        modified: Option<SystemTime>,
    }

    fn identity(meta: &Metadata) -> Identity {
        Identity {
            len: meta.len(),
            modified: meta.modified().ok(),
        }
    }

    /// Symbolic links and other reparse points.
    fn is_link(meta: &Metadata) -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return true;
            }
        }
        meta.file_type().is_symlink()
    }

    pub(super) struct Handle {
        path: PathBuf,
        file: File,
        before: Identity,
        permissions: Permissions,
    }

    pub(super) fn open(
        root: &Path,
        relative: &Path,
    ) -> std::io::Result<Result<(Handle, Vec<u8>), Refusal>> {
        let canonical_root = root.canonicalize()?;
        let mut current = canonical_root.clone();
        let mut names: Vec<_> = relative.iter().collect();
        let Some(name) = names.pop() else {
            return Ok(Err(Refusal::OutsideRoot));
        };
        for directory in names {
            current.push(directory);
            let meta = std::fs::symlink_metadata(&current)?;
            if is_link(&meta) || !meta.is_dir() {
                return Ok(Err(Refusal::Symlink));
            }
        }
        if !current.canonicalize()?.starts_with(&canonical_root) {
            return Ok(Err(Refusal::OutsideRoot));
        }
        let path = current.join(name);
        let named = std::fs::symlink_metadata(&path)?;
        if is_link(&named) {
            return Ok(Err(Refusal::Symlink));
        }
        if !named.is_file() {
            return Ok(Err(Refusal::NotRegular));
        }
        if named.len() > MAX_FILE_BYTES {
            return Ok(Err(Refusal::TooLarge));
        }
        let mut file = OpenOptions::new().read(true).open(&path)?;
        let held = file.metadata()?;
        if identity(&held) != identity(&named) {
            return Ok(Err(Refusal::Changed));
        }
        let mut bytes = Vec::new();
        Read::take(&mut file, MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        Ok(Ok((
            Handle {
                path,
                file,
                before: identity(&held),
                permissions: held.permissions(),
            },
            bytes,
        )))
    }

    struct Temp {
        path: PathBuf,
        armed: bool,
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            if self.armed {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }

    impl Handle {
        pub(super) fn commit(
            &mut self,
            text: &str,
            original: &[u8],
            hook: Hook<'_>,
        ) -> std::io::Result<Option<Refusal>> {
            hook(Step::Create)?;
            let dir = self
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            let mut created = None;
            for attempt in 0..16u32 {
                let candidate = dir.join(format!(".slopvac-fix-{}-{attempt}", std::process::id()));
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&candidate)
                {
                    Ok(file) => {
                        created = Some((file, candidate));
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
            }
            let Some((mut temp_file, path)) = created else {
                return Err(std::io::Error::other("no free temporary name"));
            };
            let mut temp = Temp { path, armed: true };
            hook(Step::Write)?;
            temp_file.write_all(text.as_bytes())?;
            hook(Step::Flush)?;
            temp_file.set_permissions(self.permissions.clone())?;
            temp_file.sync_all()?;
            hook(Step::Verify)?;
            if !self.unchanged(original)? {
                return Ok(Some(Refusal::Changed));
            }
            hook(Step::Rename)?;
            drop(temp_file);
            std::fs::rename(&temp.path, &self.path)?;
            temp.armed = false;
            Ok(None)
        }

        fn unchanged(&mut self, original: &[u8]) -> std::io::Result<bool> {
            let named = match std::fs::symlink_metadata(&self.path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(e),
            };
            if is_link(&named) || !named.is_file() {
                return Ok(false);
            }
            if identity(&named) != self.before || identity(&self.file.metadata()?) != self.before {
                return Ok(false);
            }
            self.file.rewind()?;
            let mut now = Vec::with_capacity(original.len());
            Read::take(&mut self.file, MAX_FILE_BYTES + 1).read_to_end(&mut now)?;
            Ok(now == original)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{directory_trusted, owner_trusted};

    #[test]
    fn a_directory_is_trusted_when_only_its_owner_can_write() {
        for (mode, owner, me, want) in [
            (0o755, 1000, 1000, true),
            (0o700, 1000, 1000, true),
            (0o775, 1000, 1000, false),
            (0o757, 1000, 1000, false),
            (0o1777, 1000, 1000, true),
            (0o1777, 0, 1000, false),
            (0o1775, 1000, 1000, true),
        ] {
            assert_eq!(directory_trusted(mode, owner, me), want, "{mode:o}");
        }
    }

    #[test]
    fn only_the_current_user_s_files_are_fixed() {
        assert!(owner_trusted(1000, 1000));
        assert!(
            !owner_trusted(1000, 0),
            "root does not fix another user's file"
        );
        assert!(!owner_trusted(0, 1000));
    }
}
