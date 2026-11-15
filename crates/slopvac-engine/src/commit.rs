//! The platform layer of [`fix_file`](crate::fix_file): confined opens and an atomic commit.
//!
//! This file depends on nothing else in the crate, so it can be compiled alone for another
//! target. Every open goes through a [`cap_std::fs::Dir`] handle, which resolves a name
//! relative to a directory handle instead of through the path, on unix with `openat` and on
//! Windows with handle-relative NT opens. `cap-std` is the same crate the plugin host uses
//! for its own file writes. It keeps the unsafe calls out of this crate.

use std::ffi::{OsStr, OsString};
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::time::SystemTime;

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, File, Metadata, OpenOptions, Permissions};

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
    /// Setting its group and mode and flushing it.
    Flush,
    /// Re-checking the original. Tests change the file here.
    Verify,
    /// Renaming over the original. Tests change the temporary name here.
    Rename,
}

pub(crate) type Hook<'a> = &'a mut dyn FnMut(Step) -> std::io::Result<()>;

/// What names a file. The id is the device and inode on unix, and the volume and file index
/// on Windows; a name-based lookup on Windows has none.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    id: Option<(u64, u64)>,
    size: u64,
    modified: Option<SystemTime>,
}

impl Identity {
    /// Whether both name one file: equal ids when both have one, else equal size and time.
    fn same(&self, other: &Self) -> bool {
        self.size == other.size
            && self.modified == other.modified
            && match (self.id, other.id) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}

#[cfg(unix)]
fn identity(_: &File, meta: &Metadata) -> std::io::Result<Identity> {
    use cap_std::fs::MetadataExt;
    Ok(Identity {
        id: Some((meta.dev(), meta.ino())),
        size: meta.len(),
        modified: meta
            .modified()
            .ok()
            .map(cap_std::time::SystemTime::into_std),
    })
}

#[cfg(windows)]
fn identity(file: &File, meta: &Metadata) -> std::io::Result<Identity> {
    let std_file = file.try_clone()?.into_std();
    let info = winapi_util::file::information(&std_file)?;
    Ok(Identity {
        id: Some((info.volume_serial_number(), info.file_index())),
        size: meta.len(),
        modified: meta
            .modified()
            .ok()
            .map(cap_std::time::SystemTime::into_std),
    })
}

/// The identity of a directory entry looked up by name.
fn named_identity(meta: &Metadata) -> Identity {
    #[cfg(unix)]
    let id = {
        use cap_std::fs::MetadataExt;
        Some((meta.dev(), meta.ino()))
    };
    #[cfg(not(unix))]
    let id = None;
    Identity {
        id,
        size: meta.len(),
        modified: meta
            .modified()
            .ok()
            .map(cap_std::time::SystemTime::into_std),
    }
}

#[cfg(unix)]
fn link_count(_: &File, meta: &Metadata) -> std::io::Result<u64> {
    use cap_std::fs::MetadataExt;
    Ok(meta.nlink())
}

#[cfg(windows)]
fn link_count(file: &File, _: &Metadata) -> std::io::Result<u64> {
    let std_file = file.try_clone()?.into_std();
    Ok(winapi_util::file::information(&std_file)?.number_of_links())
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

/// An open file with the directory it was found in.
pub(crate) struct Handle {
    parent: Dir,
    name: OsString,
    file: File,
    before: Identity,
    permissions: Permissions,
    #[cfg(unix)]
    group: u32,
}

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
        .map_err(|_| std::io::Error::other("O_NONBLOCK does not fit"))?;
    options.custom_flags(flag);
    Ok(())
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn nonblocking(_: &mut OpenOptions) -> std::io::Result<()> {
    Ok(())
}

/// Open `relative` below `root` without following a link, check it and read it.
pub(crate) fn open(
    root: &Path,
    relative: &Path,
) -> std::io::Result<Result<(Handle, Vec<u8>), Refusal>> {
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
    let mut file = match parent.open_with(name, &options) {
        Ok(file) => file,
        Err(e) => return link_or_error(&parent, name, e).map(Err),
    };
    let held = file.metadata()?;
    if !held.is_file() {
        return Ok(Err(Refusal::NotRegular));
    }
    if link_count(&file, &held)? > 1 {
        return Ok(Err(Refusal::Hardlinked));
    }
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if !owner_trusted(held.uid(), rustix::process::geteuid().as_raw()) {
            return Ok(Err(Refusal::NotOwner));
        }
    }
    if held.len() > MAX_FILE_BYTES {
        return Ok(Err(Refusal::TooLarge));
    }
    let before = identity(&file, &held)?;
    let bytes = read_all(&mut file)?;
    Ok(Ok((
        Handle {
            parent,
            name: name.to_owned(),
            file,
            before,
            permissions: held.permissions(),
            #[cfg(unix)]
            group: {
                use cap_std::fs::MetadataExt;
                held.gid()
            },
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
        #[cfg(unix)]
        {
            use cap_std::fs::MetadataExt;
            let dir = self.parent.dir_metadata()?;
            let me = rustix::process::geteuid().as_raw();
            if !directory_trusted(dir.mode() & 0o7777, dir.uid(), me) {
                return Ok(Some(Refusal::UnsafeDirectory));
            }
        }
        hook(Step::Create)?;
        // Duplicate the directory handle first, so a failure cannot leave the temp file.
        let guard_dir = self.parent.try_clone()?;
        let (mut temp_file, mut temp) = self.create_temp(guard_dir)?;
        hook(Step::Write)?;
        temp_file.write_all(text.as_bytes())?;
        hook(Step::Flush)?;
        if !self.match_group(&temp_file)? {
            return Ok(Some(Refusal::GroupMismatch));
        }
        temp_file.set_permissions(self.permissions.clone())?;
        temp_file.sync_all()?;
        let staged = identity(&temp_file, &temp_file.metadata()?)?;
        hook(Step::Verify)?;
        if !self.unchanged(original)? {
            return Ok(Some(Refusal::Changed));
        }
        // Windows cannot rename a file while a handle to it is open.
        drop(temp_file);
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
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
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

    /// Give the replacement the original's group, or report that it cannot. A group the
    /// current user is not in would stay with the directory's group and could widen access.
    #[cfg(unix)]
    fn match_group(&self, temp: &File) -> std::io::Result<bool> {
        use cap_std::fs::MetadataExt;
        if temp.metadata()?.gid() == self.group {
            return Ok(true);
        }
        if std::os::unix::fs::fchown(temp, None, Some(self.group)).is_err() {
            return Ok(false);
        }
        Ok(temp.metadata()?.gid() == self.group)
    }

    #[cfg(not(unix))]
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn match_group(&self, _: &File) -> std::io::Result<bool> {
        Ok(true)
    }

    /// Whether the name still leads to the file we read, unchanged and singly linked.
    fn unchanged(&mut self, original: &[u8]) -> std::io::Result<bool> {
        let held = self.file.metadata()?;
        if !identity(&self.file, &held)?.same(&self.before) || link_count(&self.file, &held)? > 1 {
            return Ok(false);
        }
        let named = match self.parent.symlink_metadata(&self.name) {
            Ok(named) => named,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        if !named.is_file() || !named_identity(&named).same(&self.before) {
            return Ok(false);
        }
        self.file.rewind()?;
        Ok(read_all(&mut self.file)? == original)
    }

    /// Whether the temporary name still holds the file we staged: the same file where an id
    /// exists, and in every case the same bytes.
    fn staged_is_ours(&self, name: &OsStr, staged: &Identity, text: &str) -> std::io::Result<bool> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        nonblocking(&mut options)?;
        let mut file = match self.parent.open_with(name, &options) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Ok(false),
        };
        let meta = file.metadata()?;
        if !meta.is_file() || !identity(&file, &meta)?.same(staged) {
            return Ok(false);
        }
        Ok(read_all(&mut file)? == text.as_bytes())
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
    fn only_the_current_users_files_are_fixed() {
        assert!(owner_trusted(1000, 1000));
        assert!(
            !owner_trusted(1000, 0),
            "root does not fix another user's file"
        );
        assert!(!owner_trusted(0, 1000));
    }
}
