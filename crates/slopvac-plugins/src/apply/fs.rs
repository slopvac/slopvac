//! File-system steps shared by edits and plans. All access goes through a
//! `cap-std` directory handle for the project root.

use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};

use crate::hooks::resolve_in_root;

use super::{ApplyError, WriteOutcome};

/// A target split into the directory that holds it and its file name.
pub(crate) struct Located {
    root: Dir,
    /// Parent directory relative to the root, with symlinks resolved.
    parent_rel: PathBuf,
    /// File name.
    pub name: OsString,
    /// Root-relative path with `/` separators and symlinked parents resolved.
    pub resolved: String,
}

/// Characters and forms that Windows reads as something other than a name.
fn windows_unsafe(seg: &str) -> bool {
    seg.contains([':', '<', '>', '"', '|', '?', '*'])
}

/// Locate `rel` under `root` and check the route stays inside the root.
///
/// The final component is not followed here. The host checks in
/// [`resolve_in_root`] refuse a link that leaves the root, anywhere in the path.
pub(crate) fn locate(root: &Path, rel: &str) -> Result<Located, ApplyError> {
    if rel.split('/').any(windows_unsafe) {
        return Err(ApplyError::PathRefused(format!(
            "`{rel}`: stream or reserved character"
        )));
    }
    resolve_in_root(root, rel).map_err(ApplyError::PathRefused)?;
    let canon = root
        .canonicalize()
        .map_err(|e| ApplyError::io(root.display(), &e))?;
    let (parent_str, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    let parent_abs = if parent_str.is_empty() {
        canon.clone()
    } else {
        resolve_in_root(root, parent_str).map_err(ApplyError::PathRefused)?
    };
    let parent_rel = parent_abs
        .strip_prefix(&canon)
        .map_err(|_| ApplyError::PathRefused(format!("`{rel}` leaves the project root")))?
        .to_owned();
    let mut resolved: Vec<String> = parent_rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    resolved.push(name.to_owned());
    let dir = Dir::open_ambient_dir(&canon, ambient_authority())
        .map_err(|e| ApplyError::io(canon.display(), &e))?;
    Ok(Located {
        root: dir,
        parent_rel,
        name: OsString::from(name),
        resolved: resolved.join("/"),
    })
}

impl Located {
    /// Open the parent directory through the root handle.
    pub(crate) fn open_parent(&self, create: bool) -> Result<Dir, ApplyError> {
        self.try_open_parent(create)?
            .ok_or_else(|| ApplyError::Io(format!("{}: directory is missing", self.resolved)))
    }

    /// Like [`Self::open_parent`], but a missing directory gives `Ok(None)`
    /// unless `create` is set.
    pub(crate) fn try_open_parent(&self, create: bool) -> Result<Option<Dir>, ApplyError> {
        let fail = |e: std::io::Error| ApplyError::io(&self.resolved, &e);
        if self.parent_rel.as_os_str().is_empty() {
            return self.root.try_clone().map(Some).map_err(fail);
        }
        if create {
            self.root.create_dir_all(&self.parent_rel).map_err(fail)?;
        }
        match self.root.open_dir(&self.parent_rel) {
            Ok(d) => Ok(Some(d)),
            Err(e) if e.kind() == ErrorKind::NotFound && !create => Ok(None),
            Err(e) => Err(fail(e)),
        }
    }
}

/// What a target holds right now.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Current {
    Absent,
    File(Vec<u8>),
}

#[cfg(unix)]
fn nofollow(opts: &mut OpenOptions) -> Result<(), ApplyError> {
    use cap_std::fs::OpenOptionsExt;
    let flag = i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())
        .map_err(|_| ApplyError::Io("O_NOFOLLOW does not fit".into()))?;
    opts.custom_flags(flag);
    Ok(())
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn nofollow(_: &mut OpenOptions) -> Result<(), ApplyError> {
    Ok(())
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn link_count(_: &cap_std::fs::File, m: &cap_std::fs::Metadata) -> std::io::Result<u64> {
    use cap_std::fs::MetadataExt;
    Ok(m.nlink())
}

#[cfg(windows)]
fn link_count(f: &cap_std::fs::File, _: &cap_std::fs::Metadata) -> std::io::Result<u64> {
    let std_file = f.try_clone()?.into_std();
    Ok(winapi_util::file::information(&std_file)?.number_of_links())
}

/// Read the target without following a final symlink.
///
/// Refuses a symlink, a non-regular file and a file with several hard links.
pub(crate) fn read_current(parent: &Dir, name: &OsStr, shown: &str) -> Result<Current, ApplyError> {
    match parent.symlink_metadata(name) {
        Ok(m) if m.file_type().is_symlink() => return Err(ApplyError::Symlink(shown.into())),
        Ok(m) if !m.is_file() => return Err(ApplyError::NotRegularFile(shown.into())),
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Current::Absent),
        Err(e) => return Err(ApplyError::io(shown, &e)),
    }
    let mut opts = OpenOptions::new();
    opts.read(true);
    nofollow(&mut opts)?;
    let mut file = match parent.open_with(name, &opts) {
        Ok(f) => f,
        Err(e) => {
            // The name may have become a symlink after the check above.
            let swapped = parent
                .symlink_metadata(name)
                .is_ok_and(|m| m.file_type().is_symlink());
            return Err(if swapped {
                ApplyError::Symlink(shown.into())
            } else {
                ApplyError::io(shown, &e)
            });
        }
    };
    let meta = file.metadata().map_err(|e| ApplyError::io(shown, &e))?;
    if !meta.is_file() {
        return Err(ApplyError::NotRegularFile(shown.into()));
    }
    let links = link_count(&file, &meta).map_err(|e| ApplyError::io(shown, &e))?;
    if links > 1 {
        return Err(ApplyError::Hardlinked(shown.into()));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| ApplyError::io(shown, &e))?;
    Ok(Current::File(bytes))
}

/// What the target must hold for a write to go ahead.
pub(crate) enum Expect<'a> {
    /// These exact bytes.
    Bytes(&'a [u8]),
    /// No file.
    Absent,
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Create a fresh temporary file next to the target.
fn create_temp(parent: &Dir, shown: &str) -> Result<(OsString, cap_std::fs::File), ApplyError> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    for _ in 0..100 {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = OsString::from(format!(".slopvac-{}-{stamp}-{n}.tmp", std::process::id()));
        match parent.open_with(&name, &opts) {
            Ok(f) => return Ok((name, f)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(ApplyError::io(shown, &e)),
        }
    }
    Err(ApplyError::Io(format!("{shown}: no free temporary name")))
}

/// Replace or create the target with `new`, atomically.
///
/// Steps: read the target (no-follow, regular, one link), compare it with
/// `expect`, write and sync a temporary file with the original mode, read the
/// target again, and rename over it only if it is unchanged. `before_commit`
/// runs after the temporary file is complete and before the second read.
pub(crate) fn replace(
    parent: &Dir,
    name: &OsStr,
    shown: &str,
    expect: &Expect<'_>,
    new: &[u8],
    before_commit: impl FnOnce(),
) -> Result<WriteOutcome, ApplyError> {
    let first = read_current(parent, name, shown)?;
    let changed = || ApplyError::SourceChanged(shown.into());
    match (expect, &first) {
        (Expect::Bytes(want), Current::File(have)) if have == want => {}
        (Expect::Absent, Current::Absent) => {}
        _ => return Err(changed()),
    }
    if matches!(&first, Current::File(have) if have == new) {
        return Ok(WriteOutcome::Unchanged);
    }
    let (tmp, file) = create_temp(parent, shown)?;
    let finish = |mut file: cap_std::fs::File| -> Result<WriteOutcome, ApplyError> {
        let io = |e: std::io::Error| ApplyError::io(shown, &e);
        file.write_all(new).map_err(io)?;
        if let Ok(meta) = parent.metadata(name) {
            file.set_permissions(meta.permissions()).map_err(io)?;
        }
        file.sync_all().map_err(io)?;
        // Windows cannot rename a file while a handle to it is open.
        drop(file);
        before_commit();
        if read_current(parent, name, shown)? != first {
            return Err(changed());
        }
        parent.rename(&tmp, parent, name).map_err(io)?;
        Ok(match first {
            Current::Absent => WriteOutcome::Created,
            Current::File(_) => WriteOutcome::Replaced,
        })
    };
    let result = finish(file);
    if result.is_err() {
        let _ = parent.remove_file(&tmp);
    }
    result
}

/// Delete the target if it is a regular file with one link.
pub(crate) fn remove(parent: &Dir, name: &OsStr, shown: &str) -> Result<WriteOutcome, ApplyError> {
    match read_current(parent, name, shown)? {
        Current::Absent => Ok(WriteOutcome::Absent),
        Current::File(_) => {
            parent
                .remove_file(name)
                .map_err(|e| ApplyError::io(shown, &e))?;
            Ok(WriteOutcome::Removed)
        }
    }
}

/// An entry of `parent` whose name equals `name` except for case.
pub(crate) fn case_twin(parent: &Dir, name: &OsStr) -> Option<String> {
    let want = name.to_string_lossy().to_lowercase();
    parent.entries().ok()?.flatten().find_map(|entry| {
        let have = entry.file_name();
        (have != name && have.to_string_lossy().to_lowercase() == want)
            .then(|| have.to_string_lossy().into_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(path: &Path) -> Dir {
        Dir::open_ambient_dir(path, ambient_authority()).unwrap()
    }

    #[test]
    fn tm15_a_rename_race_before_the_commit_loses_nothing_and_leaves_no_temporary_file() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("a.md");
        std::fs::write(&target, "original").unwrap();
        let parent = dir(tmp.path());
        let got = replace(
            &parent,
            OsStr::new("a.md"),
            "a.md",
            &Expect::Bytes(b"original"),
            b"ours",
            || std::fs::write(&target, "racer").unwrap(),
        );
        assert_eq!(got, Err(ApplyError::SourceChanged("a.md".into())));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "racer");
        let left: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn tm15_a_file_created_by_a_racer_is_not_overwritten_by_a_create() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("new.md");
        let parent = dir(tmp.path());
        let got = replace(
            &parent,
            OsStr::new("new.md"),
            "new.md",
            &Expect::Absent,
            b"ours",
            || std::fs::write(&target, "racer").unwrap(),
        );
        assert_eq!(got, Err(ApplyError::SourceChanged("new.md".into())));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "racer");
    }

    #[cfg(unix)]
    #[test]
    fn tm15_a_target_swapped_for_a_symlink_before_the_commit_is_not_replaced_through() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("a.md");
        std::fs::write(&target, "original").unwrap();
        std::fs::write(tmp.path().join("other"), "other").unwrap();
        let parent = dir(tmp.path());
        let got = replace(
            &parent,
            OsStr::new("a.md"),
            "a.md",
            &Expect::Bytes(b"original"),
            b"ours",
            || {
                std::fs::remove_file(&target).unwrap();
                std::os::unix::fs::symlink("other", &target).unwrap();
            },
        );
        assert_eq!(got, Err(ApplyError::Symlink("a.md".into())));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("other")).unwrap(),
            "other"
        );
    }
}
