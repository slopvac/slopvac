//! Capped reads and atomic writes.

use std::collections::hash_map::RandomState;
use std::ffi::{OsStr, OsString};
use std::hash::{BuildHasher, Hasher};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, OpenOptions, OpenOptionsExt, Permissions};

use crate::{Error, Result};

/// Read all of `reader`, failing when it holds more than `cap` bytes.
///
/// Memory stays bounded by `cap + 1` bytes, so an endless reader cannot exhaust it.
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

/// Options for [`write_atomic`].
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOptions {
    /// Permission bits for a file that does not exist yet. `None` keeps the owner-only `0o600`
    /// of the temporary file. An existing file keeps its own permissions either way.
    pub new_file_mode: Option<u32>,
    /// Create missing parent directories first.
    pub create_parents: bool,
}

/// How many random temporary names to try before giving up.
const TEMP_ATTEMPTS: usize = 16;

/// Replace the file at `rel` below `root` with `bytes`, so a reader sees the old content or
/// the new, never a mix.
///
/// Every step goes through a directory handle and never follows a symlink:
///
/// - `rel` must be a plain relative path: normal components and `.`, no `..`, no root. Each
///   parent directory is opened without following links, so a symlinked ancestor fails the
///   write and nothing lands outside `root`. (`root` itself is whatever the caller opened.)
/// - A destination that is a symlink, a directory or another non-regular file is refused with
///   [`Error::UnsafeTarget`]; the link is neither followed nor replaced. This matches the
///   engine's commit path, which also refuses links.
/// - The bytes go to a new file in the destination's directory, with a random name and
///   exclusive creation. The file is synced and renamed over the destination within that
///   directory, and then the directory is synced so the rename survives a crash.
/// - An existing destination keeps its permission bits, read without following links. The
///   temporary file is removed on failure.
pub fn write_atomic(root: &Dir, rel: &Path, bytes: &[u8], opts: WriteOptions) -> Result<()> {
    let io = |op, source: std::io::Error| Error::io(op, rel, &source);
    let unsafe_target = |reason| Error::UnsafeTarget {
        reason,
        path: rel.to_path_buf(),
    };
    let mut names: Vec<&OsStr> = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(name) => names.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(unsafe_target(
                    "path must be relative and stay below the root",
                ));
            }
        }
    }
    let Some((name, ancestors)) = names.split_last() else {
        return Err(unsafe_target("path names no file"));
    };
    let parent = open_parent(root, ancestors, opts.create_parents)
        .map_err(|e| io("open directory of", e))?;

    let existing = match parent.symlink_metadata(name) {
        Ok(meta) if meta.is_symlink() => return Err(unsafe_target("destination is a symlink")),
        Ok(meta) if !meta.is_file() => {
            return Err(unsafe_target("destination is not a regular file"));
        }
        Ok(meta) => Some(meta.permissions()),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(io("stat", e)),
    };
    let perms = existing.or_else(|| {
        opts.new_file_mode
            .map(|mode| Permissions::from_std(std::fs::Permissions::from_mode(mode)))
    });

    let (mut file, temp) =
        create_temp(&parent, name).map_err(|e| io("create temporary file for", e))?;
    let staged = write_to(&mut file, bytes)
        .and_then(|()| perms.map_or(Ok(()), |p| file.set_permissions(p)))
        .and_then(|()| file.sync_all())
        .map_err(|e| io("write", e));
    let finished = staged.and_then(|()| {
        parent
            .rename(&temp, &parent, name)
            .map_err(|e| io("replace", e))
    });
    if let Err(e) = finished {
        // Best effort: the original error is the one to report.
        let _ = parent.remove_file(&temp);
        return Err(e);
    }
    sync_dir(&parent).map_err(|e| io("sync directory of", e))
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
    fn debug_display_and_chain_are_path_free() {
        let cause = std::io::Error::other("boom at /secret/dir/f");
        let err = Error::io("write", Path::new("/secret/dir/f"), &cause);
        let text = format!("{err:?} | {err} | {}", chain(&err));
        assert!(!text.contains("/secret"), "{text}");
        assert!(!text.contains("boom"), "{text}");
        let err = Error::UnsafeTarget {
            reason: "destination is a symlink",
            path: PathBuf::from("/secret/dir/f"),
        };
        assert!(!format!("{err:?} {err}").contains("/secret"));
        assert_eq!(err.path(), Some(Path::new("/secret/dir/f")));
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
}
