//! The platform layer of [`fix_file`](crate::fix_file): confined opens and an atomic commit.
//!
//! This file depends on nothing else in the crate, so it can be compiled alone for another
//! target. Every open goes through a [`cap_std::fs::Dir`] handle, which resolves a name
//! relative to a directory handle instead of through the path, on unix with `openat`. It
//! keeps the unsafe calls out of this crate. `cap-std` is the same crate the plugin host
//! uses for its own file writes.
//!
//! In-place writes exist on unix only. On other platforms reading works, so a dry run and a
//! diff do, and a write is refused with [`Refusal::UnsupportedPlatform`]. A safe Windows
//! write needs a handle-relative rename and a way to keep the original's ACL, and neither
//! exists in safe Rust today (bead slopvac-8l4.3.35).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, File, OpenOptions};

/// The largest file [`fix_file`](crate::fix_file) reads.
pub const MAX_FILE_BYTES: u64 = slopvac_fs::MAX_DOCUMENT_BYTES;

/// Why [`fix_file`](crate::fix_file) left a file alone. A refused file is never read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// The path is outside the project root.
    OutsideRoot,
    /// The file or a directory above it is a symbolic link or other reparse point.
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
    /// The replacement could not get the original's group, and a different group would
    /// change who can read the file.
    GroupMismatch,
    /// The file has an access ACL, or its directory a default ACL. A replacement would get
    /// different permissions than the original.
    AclPresent,
    /// This platform has no safe in-place write. A dry run and a diff still work.
    UnsupportedPlatform,
    /// The file changed, or its name was swapped, between the read and the commit.
    Changed,
}

impl Refusal {
    /// The stable code of the refusal, in kebab-case. `document-too-large` is the code the
    /// input size caps use for [`Refusal::TooLarge`]. The other refusals each have a code
    /// that starts with `fix-`.
    #[must_use]
    pub fn code(self) -> &'static str {
        use crate::messages::FixRefusal;
        use slopvac_messages::Catalog;
        match self {
            Refusal::OutsideRoot => FixRefusal::OutsideRoot.code(),
            Refusal::Symlink => FixRefusal::Symlink.code(),
            Refusal::NotRegular => FixRefusal::NotRegular.code(),
            Refusal::Hardlinked => FixRefusal::Hardlinked.code(),
            Refusal::NotOwner => FixRefusal::NotOwner.code(),
            Refusal::TooLarge => slopvac_config::Code::DocumentTooLarge.as_str(),
            Refusal::UnsafeDirectory => FixRefusal::UnsafeDirectory.code(),
            Refusal::GroupMismatch => FixRefusal::GroupMismatch.code(),
            Refusal::AclPresent => FixRefusal::AclPresent.code(),
            Refusal::UnsupportedPlatform => FixRefusal::UnsupportedPlatform.code(),
            Refusal::Changed => FixRefusal::Changed.code(),
        }
    }
}

/// Where in a write a hook runs, before the step it names.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Creating the temporary file.
    Create,
    /// Writing the fixed text to it.
    Write,
    /// Giving it the original's group. An error here fails the assignment.
    Group,
    /// Setting its mode and flushing it.
    Flush,
    /// Re-checking the original. Tests change the file here.
    Verify,
    /// Renaming over the original. Tests change the temporary name here.
    Rename,
}

pub(crate) type Hook<'a> = &'a mut dyn FnMut(Step) -> std::io::Result<()>;

/// A directory entry that is a link, or not a plain directory, in a place that must be one.
fn link_or_error(dir: &Dir, name: &OsStr, error: std::io::Error) -> std::io::Result<Refusal> {
    match dir.symlink_metadata(name) {
        Ok(_) => Ok(Refusal::Symlink),
        Err(_) => Err(error),
    }
}

/// The cap on a file this module reads or writes: [`MAX_FILE_BYTES`], except in unit tests,
/// which lower it so a few bytes cross it.
fn max_bytes() -> u64 {
    #[cfg(test)]
    {
        seam::cap()
    }
    #[cfg(not(test))]
    {
        MAX_FILE_BYTES
    }
}

/// The size the metadata check sees for a file of `len` bytes. In unit tests a seam can pin it,
/// which stands in for a file that grew after the check and before the read.
fn checked_len(len: u64) -> u64 {
    #[cfg(test)]
    {
        seam::stale_len().unwrap_or(len)
    }
    #[cfg(not(test))]
    {
        len
    }
}

/// Whether `text` fits in a file under the cap.
fn fits(text: &str) -> bool {
    u64::try_from(text.len()).is_ok_and(|len| len <= max_bytes())
}

#[cfg(test)]
mod seam {
    use std::cell::Cell;

    thread_local! {
        static CAP: Cell<u64> = const { Cell::new(super::MAX_FILE_BYTES) };
        static STALE_LEN: Cell<Option<u64>> = const { Cell::new(None) };
    }

    pub(super) fn cap() -> u64 {
        CAP.with(Cell::get)
    }

    pub(super) fn stale_len() -> Option<u64> {
        STALE_LEN.with(Cell::get)
    }

    /// Lower the cap, and optionally pin the size the metadata check sees, until dropped.
    pub(super) struct Guard;

    pub(super) fn set(cap: u64, stale_len: Option<u64>) -> Guard {
        CAP.with(|c| c.set(cap));
        STALE_LEN.with(|c| c.set(stale_len));
        Guard
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            CAP.with(|c| c.set(super::MAX_FILE_BYTES));
            STALE_LEN.with(|c| c.set(None));
        }
    }
}

/// Read the open file, or `None` when it holds more than [`MAX_FILE_BYTES`]. The size check
/// on the metadata can be out of date by the time the read happens, so the read enforces the
/// cap itself.
fn read_all(file: &mut File) -> std::io::Result<Option<Vec<u8>>> {
    match slopvac_fs::read_capped(file, max_bytes()) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(slopvac_fs::Error::CapExceeded { .. }) => Ok(None),
        Err(slopvac_fs::Error::Read { source, .. }) => Err(source),
        Err(e) => Err(std::io::Error::other(e.to_string())),
    }
}

#[cfg(unix)]
fn nonblocking(options: &mut OpenOptions) -> std::io::Result<()> {
    use cap_std::fs::OpenOptionsExt;
    // A FIFO swapped in after the check must not block the open.
    let flag = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits())
        .map_err(|_| std::io::Error::other("O_NONBLOCK does not fit"))?;
    options.custom_flags(flag);
    Ok(())
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn nonblocking(_: &mut OpenOptions) -> std::io::Result<()> {
    Ok(())
}

/// The source of the I/O error that a failed ACL check returns, so a caller can tell it
/// from other I/O errors and word it itself.
#[derive(Debug, thiserror::Error)]
#[error("the ACL check failed")]
pub struct AclCheckFailed;

/// The project root as an open directory handle.
///
/// [`Root::open`] is the only place the root is opened by pathname. Every later open, stage
/// and rename goes through the handle, so replacing the path with a link afterwards cannot
/// move a fix to another directory. The path, and its canonical form resolved once at open,
/// are kept only to turn an absolute file path into a relative one.
#[derive(Debug)]
pub struct Root {
    dir: Dir,
    path: PathBuf,
    canonical: Option<PathBuf>,
}

impl Root {
    /// Open the directory at `path` and hold it. A link at `path` is followed here, once.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when `path` is not an openable directory.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let dir = Dir::open_ambient_dir(path, ambient_authority())?;
        // Resolved here, once, so a link or a relative root still has an absolute spelling
        // to match targets against. It is never resolved again.
        let canonical = path.canonicalize().ok();
        Ok(Self {
            dir,
            path: path.to_path_buf(),
            canonical,
        })
    }

    /// The spellings of the root that an absolute target may start with: the path the root
    /// was opened under and its canonical form. Do not open either again.
    pub(crate) fn spellings(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.path.as_path()).chain(self.canonical.as_deref())
    }
}

/// Walk to the parent of `relative` below `root` without following a link, and open the
/// file for reading. The caller checks what the file is.
fn open_file(
    root: &Root,
    relative: &Path,
) -> std::io::Result<Result<(Dir, OsString, File), Refusal>> {
    let mut parent = root.dir.try_clone()?;
    let mut names: Vec<&OsStr> = relative.iter().collect();
    let Some(name) = names.pop() else {
        return Ok(Err(Refusal::OutsideRoot));
    };
    for directory in names {
        match parent.open_dir_nofollow(directory) {
            Ok(next) => parent = next,
            Err(e) => return link_or_error(&parent, directory, e).map(Err),
        }
    }
    let entry = parent.symlink_metadata(name)?;
    if entry.is_symlink() {
        return Ok(Err(Refusal::Symlink));
    }
    if !entry.is_file() {
        return Ok(Err(Refusal::NotRegular));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    nonblocking(&mut options)?;
    match parent.open_with(name, &options) {
        Ok(file) => Ok(Ok((parent, name.to_owned(), file))),
        Err(e) => link_or_error(&parent, name, e).map(Err),
    }
}

#[cfg(unix)]
pub(crate) use unix::open;

#[cfg(not(unix))]
pub(crate) use other::open;

/// Reading only: a write is refused.
#[cfg(not(unix))]
mod other {
    use std::path::Path;

    use super::{Refusal, Root, checked_len, max_bytes, open_file, read_all};

    /// Nothing to commit with: this platform does not write in place.
    pub(crate) struct Handle;

    impl Handle {
        #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
        pub(crate) fn commit(
            &mut self,
            _: &str,
            _: &[u8],
            _: super::Hook<'_>,
        ) -> std::io::Result<Option<Refusal>> {
            Ok(Some(Refusal::UnsupportedPlatform))
        }
    }

    pub(crate) fn open(
        root: &Root,
        relative: &Path,
    ) -> std::io::Result<Result<(Handle, Vec<u8>), Refusal>> {
        let (_, _, mut file) = match open_file(root, relative)? {
            Ok(opened) => opened,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Ok(Err(Refusal::NotRegular));
        }
        if checked_len(meta.len()) > max_bytes() {
            return Ok(Err(Refusal::TooLarge));
        }
        let Some(bytes) = read_all(&mut file)? else {
            return Ok(Err(Refusal::TooLarge));
        };
        Ok(Ok((Handle, bytes)))
    }
}

#[cfg(unix)]
mod unix {
    use std::ffi::{OsStr, OsString};
    use std::hash::{BuildHasher, Hasher};
    use std::io::{Seek, Write};
    use std::path::Path;
    use std::time::SystemTime;

    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{Dir, File, Metadata, MetadataExt, OpenOptions, OpenOptionsExt, Permissions};
    use rustix::io::Errno;

    use super::{
        Hook, Refusal, Root, Step, checked_len, fits, max_bytes, nonblocking, open_file, read_all,
    };

    /// What names a file: device and inode, size and modification time.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Identity {
        id: (u64, u64),
        size: u64,
        modified: Option<SystemTime>,
    }

    fn identity(meta: &Metadata) -> Identity {
        Identity {
            id: (meta.dev(), meta.ino()),
            size: meta.len(),
            modified: meta
                .modified()
                .ok()
                .map(cap_std::time::SystemTime::into_std),
        }
    }

    /// Whether a directory with this mode may hold a file we replace: nobody but the owner
    /// can write to it, or it is sticky and ours.
    pub(super) fn directory_trusted(mode: u32, owner: u32, effective_user: u32) -> bool {
        mode & 0o022 == 0 || (mode & 0o1000 != 0 && owner == effective_user)
    }

    /// Whether the current user may fix a file this user owns. Root does not fix other
    /// users' files.
    pub(super) fn owner_trusted(owner: u32, effective_user: u32) -> bool {
        owner == effective_user
    }

    /// Whether reading an ACL attribute says there is one: a value means yes, no attribute
    /// or no ACL support means no, and a value too big for the buffer means yes.
    #[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
    pub(super) fn acl_decision(read: Result<usize, Errno>) -> std::io::Result<bool> {
        match read {
            Ok(len) => Ok(len > 0),
            Err(Errno::RANGE) => Ok(true),
            Err(Errno::NODATA | Errno::NOTSUP) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Which ACL a check looks for.
    #[derive(Clone, Copy)]
    pub(super) enum Acl {
        /// An ACL on the file itself, which a replacement file would lose.
        Access,
        /// An ACL on a directory that new files in it would inherit.
        Inherited,
    }

    /// Whether `fd` carries the ACL. Linux keeps ACLs in extended attributes.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn has_acl(fd: impl std::os::fd::AsFd, which: Acl) -> std::io::Result<bool> {
        let name = match which {
            Acl::Access => "system.posix_acl_access",
            Acl::Inherited => "system.posix_acl_default",
        };
        let mut buffer = [0u8; 4096];
        acl_decision(rustix::fs::fgetxattr(fd, name, &mut buffer[..]))
    }

    /// Whether `fd` carries the ACL. macOS hides its ACL attribute from the extended
    /// attribute calls, and `acl_get_fd` is an `unsafe` foreign call that this workspace
    /// forbids. So `ls -lde`, which prints the entries under the file, answers for the path
    /// the descriptor names. An error fails the check instead of passing the file.
    #[cfg(target_os = "macos")]
    fn has_acl(fd: impl std::os::fd::AsFd, which: Acl) -> std::io::Result<bool> {
        use std::os::unix::ffi::OsStrExt;
        let path = rustix::fs::getpath(fd)?;
        // `ls` prints the name unescaped, so a newline in it would split the first line and
        // could pass for an entry. Do not guess: treat the ACL as present.
        if path.to_bytes().contains(&b'\n') {
            return Ok(true);
        }
        let output = std::process::Command::new("/bin/ls")
            .env_clear()
            .env("LC_ALL", "C")
            .stdin(std::process::Stdio::null())
            .arg("-lde")
            .arg("--")
            .arg(OsStr::from_bytes(path.to_bytes()))
            .output()?;
        if !output.status.success() {
            return Err(std::io::Error::other(super::AclCheckFailed));
        }
        Ok(lists_acl(&String::from_utf8_lossy(&output.stdout), which))
    }

    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the same signature as the platforms that read an ACL and can fail"
    )]
    fn has_acl(_: impl std::os::fd::AsFd, _: Acl) -> std::io::Result<bool> {
        Ok(false)
    }

    /// Whether `ls -lde` output lists an ACL entry (a line like `" 0: group:everyone allow
    /// read"` under the file line), and for [`Acl::Inherited`] one that new files inherit.
    #[cfg_attr(
        all(not(target_os = "macos"), not(test)),
        expect(dead_code, reason = "only the macOS ACL check parses `ls` output")
    )]
    pub(super) fn lists_acl(output: &str, which: Acl) -> bool {
        output.lines().skip(1).any(|line| {
            let Some((index, _)) = line.trim_start().split_once(": ") else {
                return false;
            };
            !index.is_empty()
                && index.chars().all(|c| c.is_ascii_digit())
                && match which {
                    Acl::Access => true,
                    Acl::Inherited => line.contains("file_inherit"),
                }
        })
    }

    /// An open file with the directory it was found in.
    pub(crate) struct Handle {
        parent: Dir,
        name: OsString,
        file: File,
        before: Identity,
        permissions: Permissions,
        group: u32,
    }

    /// Open `relative` below `root` without following a link, check it and read it.
    pub(crate) fn open(
        root: &Root,
        relative: &Path,
    ) -> std::io::Result<Result<(Handle, Vec<u8>), Refusal>> {
        let (parent, name, mut file) = match open_file(root, relative)? {
            Ok(opened) => opened,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let held = file.metadata()?;
        if !held.is_file() {
            return Ok(Err(Refusal::NotRegular));
        }
        if held.nlink() > 1 {
            return Ok(Err(Refusal::Hardlinked));
        }
        if !owner_trusted(held.uid(), rustix::process::geteuid().as_raw()) {
            return Ok(Err(Refusal::NotOwner));
        }
        if checked_len(held.len()) > max_bytes() {
            return Ok(Err(Refusal::TooLarge));
        }
        let before = identity(&held);
        let Some(bytes) = read_all(&mut file)? else {
            return Ok(Err(Refusal::TooLarge));
        };
        Ok(Ok((
            Handle {
                parent,
                name,
                file,
                before,
                permissions: held.permissions(),
                group: held.gid(),
            },
            bytes,
        )))
    }

    /// A temporary entry that is removed on drop unless it was handed over.
    struct Temp {
        parent: Dir,
        name: OsString,
        armed: bool,
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            if self.armed {
                let _ = self.parent.remove_file(&self.name);
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
        pub(crate) fn commit(
            &mut self,
            text: &str,
            original: &[u8],
            hook: Hook<'_>,
        ) -> std::io::Result<Option<Refusal>> {
            // The scan found the text within the cap, but a replacement over it would be
            // refused by the staged check, which reads at most the cap. Refuse it up front.
            if !fits(text) {
                return Ok(Some(Refusal::TooLarge));
            }
            let dir = self.parent.dir_metadata()?;
            let me = rustix::process::geteuid().as_raw();
            if !directory_trusted(dir.mode() & 0o7777, dir.uid(), me) {
                return Ok(Some(Refusal::UnsafeDirectory));
            }
            // A replacement is a new file: it gets none of the original's ACL, and the
            // directory's default ACL could add readers once the mode is applied.
            // The directory handle may be a path-only descriptor on Linux, which the
            // attribute calls reject with EBADF. A readable handle to the same directory,
            // opened relative to it, does not have that limit.
            let readable_parent = self.parent.open(".")?;
            if has_acl(&self.file, Acl::Access)? || has_acl(&readable_parent, Acl::Inherited)? {
                return Ok(Some(Refusal::AclPresent));
            }
            hook(Step::Create)?;
            // Duplicate the directory handle first, so a failure cannot leave the temp file.
            let guard_dir = self.parent.try_clone()?;
            let (mut temp_file, mut temp) = self.create_temp(guard_dir)?;
            hook(Step::Write)?;
            temp_file.write_all(text.as_bytes())?;
            if !self.match_group(&temp_file, hook)? {
                return Ok(Some(Refusal::GroupMismatch));
            }
            hook(Step::Flush)?;
            temp_file.set_permissions(self.permissions.clone())?;
            temp_file.sync_all()?;
            let staged = identity(&temp_file.metadata()?);
            hook(Step::Verify)?;
            if !self.unchanged(original)? {
                return Ok(Some(Refusal::Changed));
            }
            hook(Step::Rename)?;
            if !self.staged_is_ours(&temp.name, &staged, text)? {
                // Someone put something else under our name: leave it, and write nothing.
                temp.armed = false;
                return Ok(Some(Refusal::Changed));
            }
            self.parent.rename(&temp.name, &self.parent, &self.name)?;
            temp.armed = false;
            if let Ok(dir) = self.parent.open(".") {
                let _ = dir.sync_all();
            }
            Ok(None)
        }

        fn create_temp(&self, guard_dir: Dir) -> std::io::Result<(File, Temp)> {
            let mut options = OpenOptions::new();
            options
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No)
                .mode(0o600);
            for _ in 0..16 {
                let name = temp_name(&self.name);
                match self.parent.open_with(&name, &options) {
                    Ok(file) => {
                        let temp = Temp {
                            parent: guard_dir,
                            name,
                            armed: true,
                        };
                        return Ok((file, temp));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
            }
            Err(std::io::Error::other("no free temporary name"))
        }

        /// Give the replacement the original's group, or report that it cannot. A group
        /// the current user is not in would stay with the directory's group and could
        /// widen access. An error from the hook counts as a failed assignment.
        fn match_group(&self, temp: &File, hook: Hook<'_>) -> std::io::Result<bool> {
            if hook(Step::Group).is_err() {
                return Ok(false);
            }
            if temp.metadata()?.gid() == self.group {
                return Ok(true);
            }
            if std::os::unix::fs::fchown(temp, None, Some(self.group)).is_err() {
                return Ok(false);
            }
            Ok(temp.metadata()?.gid() == self.group)
        }

        /// Whether the name still leads to the file we read, unchanged and singly linked.
        fn unchanged(&mut self, original: &[u8]) -> std::io::Result<bool> {
            let held = self.file.metadata()?;
            if identity(&held) != self.before || held.nlink() > 1 {
                return Ok(false);
            }
            let named = match self.parent.symlink_metadata(&self.name) {
                Ok(named) => named,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(e),
            };
            if !named.is_file() || identity(&named) != self.before {
                return Ok(false);
            }
            self.file.rewind()?;
            Ok(read_all(&mut self.file)?.is_some_and(|b| b == original))
        }

        /// Whether the temporary name still holds the file we staged: the same file, with
        /// the same bytes.
        fn staged_is_ours(
            &self,
            name: &OsStr,
            staged: &Identity,
            text: &str,
        ) -> std::io::Result<bool> {
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            nonblocking(&mut options)?;
            let Ok(mut file) = self.parent.open_with(name, &options) else {
                return Ok(false);
            };
            let meta = file.metadata()?;
            if !meta.is_file() || identity(&meta) != *staged {
                return Ok(false);
            }
            Ok(read_all(&mut file)?.is_some_and(|b| b == text.as_bytes()))
        }
    }

    #[cfg(test)]
    mod tests {
        use rustix::io::Errno;

        use std::path::Path;

        use super::{Acl, acl_decision, directory_trusted, lists_acl, open, owner_trusted};
        use crate::commit::{Refusal, seam};

        fn project(text: &str) -> tempfile::TempDir {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("doc.md"), text).unwrap();
            dir
        }

        #[test]
        fn a_file_that_grows_after_the_size_check_is_refused_not_read_as_a_prefix() {
            let dir = project("12345");
            // The check sees 0 bytes, as if the file had been empty when it ran.
            let _seam = seam::set(4, Some(0));
            let opened =
                open(&super::Root::open(dir.path()).unwrap(), Path::new("doc.md")).unwrap();
            assert_eq!(opened.err(), Some(Refusal::TooLarge));
        }

        #[test]
        fn a_file_exactly_at_the_cap_is_read_whole() {
            let dir = project("1234");
            let _seam = seam::set(4, None);
            let (_, bytes) = open(&super::Root::open(dir.path()).unwrap(), Path::new("doc.md"))
                .unwrap()
                .unwrap();
            assert_eq!(bytes, b"1234");
        }

        #[test]
        fn a_replacement_over_the_cap_is_refused_before_staging() {
            let dir = project("1234");
            let _seam = seam::set(4, None);
            let (mut handle, bytes) =
                open(&super::Root::open(dir.path()).unwrap(), Path::new("doc.md"))
                    .unwrap()
                    .unwrap();
            let refusal = handle.commit("12345", &bytes, &mut |_| Ok(())).unwrap();
            assert_eq!(refusal, Some(Refusal::TooLarge));
            assert_eq!(std::fs::read(dir.path().join("doc.md")).unwrap(), b"1234");
            let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
            assert_eq!(names.len(), 1, "no temporary file is left");
        }

        #[test]
        fn a_replacement_at_the_cap_is_written() {
            let dir = project("1234");
            let _seam = seam::set(4, None);
            let (mut handle, bytes) =
                open(&super::Root::open(dir.path()).unwrap(), Path::new("doc.md"))
                    .unwrap()
                    .unwrap();
            let refusal = handle.commit("abcd", &bytes, &mut |_| Ok(())).unwrap();
            assert_eq!(refusal, None);
            assert_eq!(std::fs::read(dir.path().join("doc.md")).unwrap(), b"abcd");
        }

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
        fn only_the_current_users_files_are_fixed() {
            assert!(owner_trusted(1000, 1000));
            assert!(
                !owner_trusted(1000, 0),
                "root does not fix another user's file"
            );
            assert!(!owner_trusted(0, 1000));
        }

        #[test]
        fn an_acl_attribute_decides_by_what_the_read_returned() {
            assert!(acl_decision(Ok(28)).unwrap(), "a value is an ACL");
            assert!(
                acl_decision(Err(Errno::RANGE)).unwrap(),
                "a long value is an ACL"
            );
            assert!(!acl_decision(Ok(0)).unwrap(), "an empty value is none");
            assert!(
                !acl_decision(Err(Errno::NODATA)).unwrap(),
                "no attribute is none"
            );
            assert!(
                !acl_decision(Err(Errno::NOTSUP)).unwrap(),
                "no ACL support is none"
            );
            assert!(
                acl_decision(Err(Errno::IO)).is_err(),
                "other errors are errors"
            );
        }

        #[test]
        fn listed_entries_under_the_file_line_are_an_acl() {
            let plain = "-rw-r--r--  1 u  staff  2 Nov 21 14:33 f\n";
            let with_acl =
                "-rw-r--r--+ 1 u  staff  2 Nov 21 14:33 f\n 0: group:everyone allow read\n";
            let inheriting = "drwxr-xr-x+ 2 u  staff 64 Nov 21 14:33 d\n 0: group:everyone allow add_file,file_inherit\n";
            assert!(!lists_acl(plain, Acl::Access));
            assert!(lists_acl(with_acl, Acl::Access));
            assert!(!lists_acl(with_acl, Acl::Inherited), "not inherited");
            assert!(lists_acl(inheriting, Acl::Inherited));
            // A name that looks like an entry is on the first line, which is skipped.
            assert!(!lists_acl(" 0: x\n", Acl::Access));
        }
    }
}
