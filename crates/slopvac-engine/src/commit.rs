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
use std::io::Read;
use std::path::Path;

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, File, OpenOptions};

/// The largest file [`fix_file`](crate::fix_file) reads.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

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

fn read_all(file: &mut File) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    Read::take(file, MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(unix)]
fn nonblocking(options: &mut OpenOptions) -> std::io::Result<()> {
    use cap_std::fs::OpenOptionsExt;
    // A FIFO swapped in after the check must not block the open.
    let flag = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits())
        .map_err(|e| std::io::Error::other(format!("O_NONBLOCK does not fit: {e}")))?;
    options.custom_flags(flag);
    Ok(())
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn nonblocking(_: &mut OpenOptions) -> std::io::Result<()> {
    Ok(())
}

/// Walk to the parent of `relative` below `root` without following a link, and open the
/// file for reading. The caller checks what the file is.
fn open_file(
    root: &Path,
    relative: &Path,
) -> std::io::Result<Result<(Dir, OsString, File), Refusal>> {
    let mut parent = Dir::open_ambient_dir(root, ambient_authority())?;
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

    use super::{MAX_FILE_BYTES, Refusal, open_file, read_all};

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
        root: &Path,
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
        if meta.len() > MAX_FILE_BYTES {
            return Ok(Err(Refusal::TooLarge));
        }
        let bytes = read_all(&mut file)?;
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

    use super::{Hook, MAX_FILE_BYTES, Refusal, Step, nonblocking, open_file, read_all};

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
    #[cfg_attr(
        all(not(test), not(any(target_os = "linux", target_os = "android"))),
        expect(dead_code, reason = "only the Linux build reads ACL attributes")
    )]
    pub(super) fn acl_decision(read: Result<usize, Errno>) -> std::io::Result<bool> {
        match read {
            Ok(len) => Ok(len > 0),
            Err(Errno::RANGE) => Ok(true),
            Err(Errno::NODATA | Errno::NOTSUP) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Whether `fd` carries the ACL attribute `name`. Only Linux exposes ACLs as extended
    /// attributes. macOS keeps them in a hidden attribute and other systems elsewhere, so
    /// there an ACL is not detected.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn has_acl(fd: impl std::os::fd::AsFd, name: &str) -> std::io::Result<bool> {
        let mut buffer = [0u8; 4096];
        acl_decision(rustix::fs::fgetxattr(fd, name, &mut buffer[..]))
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the signature matches the other target, where the call can fail"
    )]
    fn has_acl(_: impl std::os::fd::AsFd, _: &str) -> std::io::Result<bool> {
        Ok(false)
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
        root: &Path,
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
        if held.len() > MAX_FILE_BYTES {
            return Ok(Err(Refusal::TooLarge));
        }
        let before = identity(&held);
        let bytes = read_all(&mut file)?;
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
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "cleanup on drop: a leftover temporary file is harmless, and a destructor has no caller to report to"
                )]
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
            if has_acl(&self.file, "system.posix_acl_access")?
                || has_acl(&readable_parent, "system.posix_acl_default")?
            {
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
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "syncing the directory is best effort after the rename has succeeded"
                )]
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
            Ok(read_all(&mut self.file)? == original)
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
            Ok(read_all(&mut file)? == text.as_bytes())
        }
    }

    #[cfg(test)]
    mod tests {
        use rustix::io::Errno;

        use super::{acl_decision, directory_trusted, owner_trusted};

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
    }
}
