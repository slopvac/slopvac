//! Capped reads and atomic writes.

use std::collections::hash_map::RandomState;
use std::ffi::{OsStr, OsString};
use std::hash::{BuildHasher, Hasher};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, MetadataExt, OpenOptions, OpenOptionsExt, Permissions};

use crate::{Error, Op, Reason, Result};

/// The most bytes of one document that any slopvac entry point reads: 64 MiB.
///
/// The span crates index text with 32-bit offsets and size their structures from its length, so
/// every path that feeds them text must stay under this. The larger caps for each entry point
/// are listed in `docs/contract/policies.md`.
pub const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

/// Read all of `reader`, failing when it holds more than `cap` bytes.
///
/// Memory stays bounded by `cap + 1` bytes, so an endless reader cannot exhaust it. The read
/// blocks for as long as the reader does, though: a FIFO or a device with no writer never
/// ends, and this function sets no timeout. It also takes whatever the reader gives, so it
/// does not check what kind of file backs it. To read a file safely, use
/// [`read_capped_regular`](crate::read_capped_regular) or
/// [`read_capped_regular_in`](crate::read_capped_regular_in), which refuse anything but a
/// regular file.
pub fn read_capped(reader: impl Read, cap: u64) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    reader
        .take(cap.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| Error::read(&e))?;
    // A length always fits in u64; the fallback keeps the check total without a cast.
    if u64::try_from(buf.len()).map_or(true, |len| len > cap) {
        return Err(Error::CapExceeded { cap });
    }
    Ok(buf)
}

/// Read the file at `path` as UTF-8 text, failing when it holds more than `cap` bytes.
///
/// Memory stays bounded by `cap + 1` bytes whatever the file reports as its size.
///
/// # Errors
/// [`Error::CapExceeded`] when the file is larger than `cap`, [`Error::NotUtf8`] when the bytes
/// are not UTF-8, and [`Error::Io`] when opening or reading fails.
pub fn read_text_capped(path: &Path, cap: u64) -> Result<String> {
    let file = std::fs::File::open(path).map_err(|e| Error::io(Op::Open, path, &e))?;
    let bytes = read_capped(file, cap).map_err(|e| match e {
        Error::Read(source) => Error::io(Op::ReadFile, path, &source),
        other => other,
    })?;
    String::from_utf8(bytes).map_err(|_| Error::NotUtf8 {
        path: path.to_path_buf(),
    })
}

/// Options for [`write_atomic`].
///
/// Build one with [`WriteOptions::new`] and the chained setters, so a field added later does
/// not break callers.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct WriteOptions {
    /// Permission bits for a file that does not exist yet. `None` keeps the owner-only `0o600`
    /// of the temporary file. An existing file keeps its own permissions either way.
    pub new_file_mode: Option<u32>,
    /// Create missing parent directories first.
    pub create_parents: bool,
}

impl WriteOptions {
    /// The defaults: a new file is owner-only and missing parent directories are an error.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            new_file_mode: None,
            create_parents: false,
        }
    }

    /// Set the permission bits of a file that does not exist yet.
    #[must_use]
    pub const fn new_file_mode(mut self, mode: u32) -> Self {
        self.new_file_mode = Some(mode);
        self
    }

    /// Create missing parent directories first, or not.
    #[must_use]
    pub const fn create_parents(mut self, create: bool) -> Self {
        self.create_parents = create;
        self
    }
}

/// How many random temporary names to try before giving up.
const TEMP_ATTEMPTS: usize = 16;

/// Split a plain relative path into its file name and the directories above it.
///
/// Accepts normal components and `.`. Rejects `..`, a root and an empty path.
pub(crate) fn split_plain(rel: &Path) -> Result<(&OsStr, Vec<&OsStr>)> {
    let mut names: Vec<&OsStr> = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(name) => names.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::unsafe_target(Reason::NotRelative, rel));
            }
        }
    }
    let Some((name, ancestors)) = names.split_last() else {
        return Err(Error::unsafe_target(Reason::NoFileName, rel));
    };
    Ok((name, ancestors.to_vec()))
}

/// Replace the file at `rel` below `root` with `bytes`, so a reader sees the old content or
/// the new, never a mix.
///
/// Every step goes through a directory handle and does not follow a symlink:
///
/// - `rel` must be a plain relative path: normal components and `.`, no `..`, no root. Each
///   parent directory is opened without following links, so a symlinked ancestor fails the
///   write and nothing lands outside `root`. (`root` itself is whatever the caller opened.)
/// - A destination that is a symlink, a directory or another non-regular file when the write
///   starts is refused with [`Error::UnsafeTarget`]. This check and the rename that follows
///   are two steps, not one: the guarantee assumes no other process can write to the
///   destination's directory meanwhile. A process that can, and swaps the destination in
///   between, gets the swapped entry replaced (the link itself, never its target). Such a
///   process is a same-user writer, residual risk R6; see the crate docs.
/// - The bytes go to a new file in the destination's directory, with a random name and
///   exclusive creation. The file is synced, then compared with its name (device and inode,
///   and that the name is a regular file), and renamed over the destination within that
///   directory. A temporary name that no longer matches fails the write and is left alone.
///   Then the directory is synced so the rename survives a crash.
/// - An existing destination keeps its permission bits, read without following links. The
///   temporary file is removed on failure.
pub fn write_atomic(root: &Dir, rel: &Path, bytes: &[u8], opts: WriteOptions) -> Result<()> {
    let io = |op, source: std::io::Error| Error::io(op, rel, &source);
    let (name, ancestors) = split_plain(rel)?;
    let parent =
        open_parent(root, &ancestors, opts.create_parents).map_err(|e| io(Op::OpenDirectory, e))?;

    let existing = match parent.symlink_metadata(name) {
        Ok(meta) if meta.is_symlink() => {
            return Err(Error::unsafe_target(Reason::DestinationSymlink, rel));
        }
        Ok(meta) if !meta.is_file() => {
            return Err(Error::unsafe_target(Reason::DestinationNotRegular, rel));
        }
        Ok(meta) => Some(meta.permissions()),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(io(Op::Stat, e)),
    };
    let perms = existing.or_else(|| {
        opts.new_file_mode
            .map(|mode| Permissions::from_std(std::fs::Permissions::from_mode(mode)))
    });

    let (mut file, temp) = create_temp(&parent, name).map_err(|e| io(Op::CreateTemp, e))?;
    let staged = write_to(&mut file, bytes)
        .and_then(|()| perms.map_or(Ok(()), |p| file.set_permissions(p)))
        .and_then(|()| file.sync_all())
        .map_err(|e| io(Op::Write, e));
    if let Err(e) = staged {
        // Best effort: the original error is the one to report.
        let _ = parent.remove_file(&temp);
        return Err(e);
    }
    match still_staged(&parent, &temp, &file) {
        Ok(true) => {}
        // The name no longer leads to our file, so it is not ours to remove.
        Ok(false) => {
            return Err(io(
                Op::VerifyStaged,
                std::io::Error::other("staged file was replaced"),
            ));
        }
        Err(e) => {
            let _ = parent.remove_file(&temp);
            return Err(io(Op::VerifyStaged, e));
        }
    }
    if let Err(e) = parent.rename(&temp, &parent, name) {
        let _ = parent.remove_file(&temp);
        return Err(io(Op::Replace, e));
    }
    sync_dir(&parent).map_err(|e| io(Op::SyncDirectory, e))
}

/// Whether `temp` in `parent` still names the open `file`: a regular file with the same
/// device and inode.
fn still_staged(parent: &Dir, temp: &OsStr, file: &File) -> std::io::Result<bool> {
    #[cfg(test)]
    tests::before_verify(temp);
    let opened = file.metadata()?;
    let named = parent.symlink_metadata(temp)?;
    Ok(named.is_file() && named.dev() == opened.dev() && named.ino() == opened.ino())
}

/// Flush the directory `dir` itself, so its entries survive a crash.
///
/// Every directory sync goes through here. Under test, a thread-local hook can fail a chosen
/// call, because a real directory sync cannot be made to fail from a test.
fn sync_dir(dir: &Dir) -> std::io::Result<()> {
    #[cfg(test)]
    tests::inject_sync_failure()?;
    dir.open(".")?.sync_all()
}

/// Write all of `bytes`. The error is the writer's own, so its OS code survives.
///
/// Under test, a thread-local hook can fail the write, because a full disk cannot be made
/// portably from a test.
fn write_to(mut writer: impl Write, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(test)]
    tests::inject_write_failure()?;
    writer.write_all(bytes)
}

/// Open the directory that holds the destination, one component at a time without following
/// links, creating missing components when asked.
fn open_parent(root: &Dir, ancestors: &[&OsStr], create: bool) -> std::io::Result<Dir> {
    let mut dir = root.try_clone()?;
    for name in ancestors {
        dir = match dir.open_dir_nofollow(name) {
            Err(e) if create && e.kind() == ErrorKind::NotFound => {
                match dir.create_dir(name) {
                    // Sync the directory that gained an entry, so the new directory survives a
                    // crash.
                    Ok(()) => sync_dir(&dir)?,
                    Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
                dir.open_dir_nofollow(name)?
            }
            other => other?,
        };
    }
    Ok(dir)
}

fn temp_name(name: &OsStr) -> OsString {
    let random = RandomState::new().build_hasher().finish();
    let mut out = OsString::from(".");
    if name.len() <= 200 {
        out.push(name);
    }
    out.push(format!(".slopvac-{random:016x}"));
    out
}

/// Create the temporary file exclusively, owner-only, without following a link.
fn create_temp(parent: &Dir, name: &OsStr) -> std::io::Result<(File, OsString)> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No)
        .mode(0o600);
    for _ in 0..TEMP_ATTEMPTS {
        let temp = temp_name(name);
        match parent.open_with(&temp, &options) {
            Ok(file) => return Ok((file, temp)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free temporary name"))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, repeat};
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    use cap_std::ambient_authority;

    use super::*;

    thread_local! {
        /// `(calls left before the failure, OS code)` for the next `sync_dir` calls.
        static SYNC_FAILURE: std::cell::Cell<Option<(usize, i32)>> =
            const { std::cell::Cell::new(None) };
    }

    thread_local! {
        /// OS code for the next `write_to` call to fail with.
        static WRITE_FAILURE: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
    }

    /// Called by `write_to`: fail once with the planned OS code.
    pub(super) fn inject_write_failure() -> std::io::Result<()> {
        WRITE_FAILURE
            .take()
            .map_or(Ok(()), |code| Err(std::io::Error::from_raw_os_error(code)))
    }

    type BeforeVerify = Box<dyn FnOnce(&OsStr)>;

    thread_local! {
        /// Runs once between staging a file and checking its name, with the temporary name.
        static BEFORE_VERIFY: std::cell::RefCell<Option<BeforeVerify>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Called by `still_staged`: run the planned hook, if any.
    pub(super) fn before_verify(temp: &OsStr) {
        if let Some(hook) = BEFORE_VERIFY.take() {
            hook(temp);
        }
    }

    /// Called by `sync_dir`: fail with the planned OS code once the countdown reaches zero.
    pub(super) fn inject_sync_failure() -> std::io::Result<()> {
        SYNC_FAILURE.with(|plan| match plan.get() {
            Some((0, code)) => {
                plan.set(None);
                Err(std::io::Error::from_raw_os_error(code))
            }
            Some((skip, code)) => {
                plan.set(Some((skip - 1, code)));
                Ok(())
            }
            None => Ok(()),
        })
    }

    #[test]
    fn read_capped_accepts_exactly_the_cap() {
        assert_eq!(read_capped(&b"abc"[..], 3).unwrap(), b"abc");
    }

    #[test]
    fn read_capped_rejects_one_over() {
        assert!(matches!(
            read_capped(&b"abcd"[..], 3),
            Err(Error::CapExceeded { cap: 3 })
        ));
    }

    #[test]
    fn read_capped_stops_an_endless_reader() {
        assert!(matches!(
            read_capped(repeat(b'x'), 1024),
            Err(Error::CapExceeded { .. })
        ));
    }

    #[test]
    fn read_text_capped_reads_a_file_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "héllo").unwrap();
        // "héllo" is 6 bytes in UTF-8.
        assert_eq!(read_text_capped(&path, 6).unwrap(), "héllo");
    }

    #[test]
    fn read_text_capped_rejects_one_byte_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "héllo").unwrap();
        assert!(matches!(
            read_text_capped(&path, 5),
            Err(Error::CapExceeded { cap: 5 })
        ));
    }

    #[test]
    fn read_text_capped_refuses_invalid_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        let err = read_text_capped(&path, 10).unwrap_err();
        assert!(matches!(err, Error::NotUtf8 { .. }));
        assert_eq!(err.path(), Some(path.as_path()));
    }

    #[test]
    fn read_text_capped_reports_a_missing_file_with_its_path() {
        let path = PathBuf::from("/nonexistent-slopvac-dir/a.txt");
        let err = read_text_capped(&path, 10).unwrap_err();
        assert!(matches!(&err, Error::Io { op: Op::Open, .. }));
        assert_eq!(err.path(), Some(path.as_path()));
    }

    struct Failing(Option<std::io::Error>);

    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(self.0.take().expect("read called once"))
        }
    }

    fn chain(err: &Error) -> String {
        let mut text = err.to_string();
        let mut src = std::error::Error::source(err);
        while let Some(e) = src {
            text.push_str(&e.to_string());
            src = e.source();
        }
        text
    }

    #[test]
    fn read_error_with_a_message_is_redacted_and_keeps_its_kind() {
        let cause = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "no access to /secret/dir/f",
        );
        let err = read_capped(Failing(Some(cause)), 8).unwrap_err();
        assert!(!chain(&err).contains("/secret"), "{}", chain(&err));
        let Error::Read(source) = err else {
            panic!("expected Error::Read")
        };
        assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(source.raw_os_error(), None);
    }

    #[test]
    fn read_error_from_an_os_code_keeps_the_code() {
        // 2 is ENOENT on every Unix.
        let err = read_capped(Failing(Some(std::io::Error::from_raw_os_error(2))), 8).unwrap_err();
        let Error::Read(source) = err else {
            panic!("expected Error::Read")
        };
        assert_eq!(source.raw_os_error(), Some(2));
        assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn read_capped_zero_cap_accepts_empty_only() {
        assert_eq!(read_capped(&b""[..], 0).unwrap(), Vec::<u8>::new());
        assert!(read_capped(&b"x"[..], 0).is_err());
    }
    fn root(path: &Path) -> Dir {
        Dir::open_ambient_dir(path, ambient_authority()).unwrap()
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn write(dir: &tempfile::TempDir, rel: &str, bytes: &[u8], opts: WriteOptions) -> Result<()> {
        write_atomic(&root(dir.path()), Path::new(rel), bytes, opts)
    }

    #[test]
    fn write_creates_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir, "f.txt", b"one", WriteOptions::default()).unwrap();
        write(&dir, "f.txt", b"two", WriteOptions::default()).unwrap();
        assert_eq!(std::fs::read(dir.path().join("f.txt")).unwrap(), b"two");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temporary file is left behind"
        );
    }

    #[test]
    fn replacement_is_atomic_for_readers() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.txt");
        write(&dir, "f.txt", b"old", WriteOptions::default()).unwrap();
        let mut before = std::fs::File::open(&p).unwrap();
        write(&dir, "f.txt", b"new content", WriteOptions::default()).unwrap();
        let mut seen = String::new();
        before.read_to_string(&mut seen).unwrap();
        assert_eq!(seen, "old", "a reader opened earlier keeps the old file");
        assert_eq!(std::fs::read(&p).unwrap(), b"new content");
    }

    #[test]
    fn io_error_hides_the_path_but_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let rel = Path::new("missing/f");
        let err = write(&dir, "missing/f", b"x", WriteOptions::default()).unwrap_err();
        assert_eq!(err.path(), Some(rel));
        let secret = dir.path().to_string_lossy().into_owned();
        let text = format!("{err:?}{}", chain(&err));
        assert!(!text.contains(&secret), "{text}");
        assert!(!text.contains("missing"), "{text}");
    }

    #[test]
    fn write_keeps_existing_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.txt");
        std::fs::write(&p, b"old").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
        let opts = WriteOptions {
            new_file_mode: Some(0o600),
            ..WriteOptions::default()
        };
        write(&dir, "f.txt", b"new", opts).unwrap();
        assert_eq!(mode(&p), 0o640);
    }

    #[test]
    fn write_new_file_modes() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir, "a", b"x", WriteOptions::default()).unwrap();
        assert_eq!(mode(&dir.path().join("a")), 0o600);
        let opts = WriteOptions {
            new_file_mode: Some(0o644),
            ..WriteOptions::default()
        };
        write(&dir, "b", b"x", opts).unwrap();
        assert_eq!(mode(&dir.path().join("b")), 0o644);
    }

    #[test]
    fn write_creates_parents_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write(&dir, "x/y/f", b"1", WriteOptions::default()).is_err());
        let opts = WriteOptions {
            create_parents: true,
            ..WriteOptions::default()
        };
        write(&dir, "x/y/f", b"1", opts).unwrap();
        assert_eq!(std::fs::read(dir.path().join("x/y/f")).unwrap(), b"1");
    }

    #[test]
    fn write_error_keeps_kind_and_os_code() {
        let dir = tempfile::tempdir().unwrap();
        let err = write(&dir, "no/such/f", b"x", WriteOptions::default()).unwrap_err();
        let Error::Io { source, .. } = err else {
            panic!("expected Error::Io")
        };
        assert_eq!(source.raw_os_error(), Some(2), "ENOENT");
        assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn unwritable_parent_keeps_eacces() {
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = write(&dir, "locked/f", b"x", WriteOptions::default());
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        if rustix::process::geteuid().is_root() {
            // Directory permissions do not apply to root.
            return;
        }
        let Err(Error::Io { source, .. }) = result else {
            panic!("expected Error::Io, got {result:?}");
        };
        assert_eq!(source.raw_os_error(), Some(13), "EACCES");
        assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn failed_data_write_keeps_the_destination_and_leaves_no_temporary_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("f"), b"old").unwrap();
        // 28 is ENOSPC on every Unix; its kind is StorageFull.
        WRITE_FAILURE.set(Some(28));
        let result = write(&tmp, "f", b"new", WriteOptions::default());
        WRITE_FAILURE.set(None);
        assert_eq!(std::fs::read(tmp.path().join("f")).unwrap(), b"old");
        let names: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [OsString::from("f")]);
        let Err(Error::Io { source, .. }) = result else {
            panic!("expected Error::Io, got {result:?}");
        };
        assert_eq!(source.raw_os_error(), Some(28));
        assert_eq!(source.kind(), std::io::ErrorKind::StorageFull);
    }

    #[test]
    fn ancestor_symlink_cannot_escape_the_root() {
        let base = tempfile::tempdir().unwrap();
        let inside = base.path().join("root");
        let outside = base.path().join("outside");
        std::fs::create_dir(&inside).unwrap();
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, inside.join("link")).unwrap();
        let err = write_atomic(
            &root(&inside),
            Path::new("link/f"),
            b"x",
            WriteOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Io { .. }), "{err:?}");
        assert_eq!(
            std::fs::read_dir(&outside).unwrap().count(),
            0,
            "nothing written outside"
        );
        let opts = WriteOptions {
            create_parents: true,
            ..WriteOptions::default()
        };
        assert!(write_atomic(&root(&inside), Path::new("link/f"), b"x", opts).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    }

    #[test]
    fn destination_symlink_is_refused_and_the_referent_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"keep").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o777)).unwrap();
        symlink(&target, dir.path().join("link")).unwrap();
        let err = write(&dir, "link", b"new", WriteOptions::default()).unwrap_err();
        assert!(matches!(err, Error::UnsafeTarget { .. }), "{err:?}");
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        assert_eq!(mode(&target), 0o777);
        assert!(
            std::fs::symlink_metadata(dir.path().join("link"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            2,
            "no temporary file"
        );
    }

    #[test]
    fn non_regular_and_unsafe_paths_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("d")).unwrap();
        for rel in ["d", "../x", "/abs/x", "", "."] {
            let err = write(&dir, rel, b"x", WriteOptions::default()).unwrap_err();
            assert!(matches!(err, Error::UnsafeTarget { .. }), "{rel}: {err:?}");
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn write_atomic_reports_a_failed_sync_of_the_directory_that_gained_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        // 28 is ENOSPC on every Unix; its kind is StorageFull.
        // The first directory sync follows the creation of `new`, in the root.
        SYNC_FAILURE.set(Some((0, 28)));
        let err = write_atomic(
            &root(tmp.path()),
            Path::new("new/f"),
            b"x",
            WriteOptions {
                create_parents: true,
                ..WriteOptions::default()
            },
        )
        .unwrap_err();
        SYNC_FAILURE.set(None);
        assert!(!tmp.path().join("new/f").exists());
        let Error::Io { source, .. } = err else {
            panic!("expected Error::Io, got {err:?}")
        };
        assert_eq!(source.raw_os_error(), Some(28));
        assert_eq!(source.kind(), std::io::ErrorKind::StorageFull);
    }

    #[test]
    fn a_staged_file_replaced_before_the_rename_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("f"), b"old").unwrap();
        std::fs::write(tmp.path().join("other"), b"planted").unwrap();
        let base = tmp.path().to_path_buf();
        // Another file takes the temporary name, as a local attacker might.
        BEFORE_VERIFY.set(Some(Box::new(move |temp| {
            std::fs::rename(base.join("other"), base.join(temp)).unwrap();
        })));
        let result = write(&tmp, "f", b"new", WriteOptions::default());
        BEFORE_VERIFY.set(None);
        assert_eq!(std::fs::read(tmp.path().join("f")).unwrap(), b"old");
        let planted: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().is_some_and(|n| n != "f"))
            .collect();
        assert_eq!(planted.len(), 1, "the replacing file is left alone");
        assert_eq!(std::fs::read(&planted[0]).unwrap(), b"planted");
        let Err(Error::Io { op, .. }) = result else {
            panic!("expected Error::Io, got {result:?}");
        };
        assert_eq!(op, Op::VerifyStaged);
    }
}
