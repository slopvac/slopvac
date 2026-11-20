//! Project file access anchored to a directory handle.
//!
//! Every read and write goes through a `cap_std` directory opened once for the
//! project root. Path resolution happens inside the kernel relative to that
//! handle, so a directory swapped for a symlink between a check and its use
//! cannot lead outside the project (CWE-59, CWE-367). Absolute symlinks and links
//! that climb out of the root fail with [`AgentsError::UnsafeTarget`].

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions, Permissions};

use crate::error::AgentsError;

/// Longest symlink chain setup follows for a steering file.
const MAX_LINKS: usize = 8;

/// A project directory opened as a capability.
pub struct Project {
    dir: Dir,
    root: PathBuf,
}

/// A point inside a write or delete where a test can change the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// The temporary file holds the new bytes, and its final mode is not set yet.
    Filled,
    /// The new state is ready, and the final check against the expected bytes
    /// comes next.
    BeforeCommit,
}

/// One step of a path being resolved.
enum Step {
    Name(OsString),
    Up,
}

/// Split a relative path into steps; `None` for an absolute path or a prefix.
fn steps(path: &Path) -> Option<VecDeque<Step>> {
    let mut out = VecDeque::new();
    for part in path.components() {
        match part {
            Component::Normal(p) => out.push_back(Step::Name(p.to_owned())),
            Component::ParentDir => out.push_back(Step::Up),
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

fn is_git(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|n| n.eq_ignore_ascii_case(".git"))
}

/// Check a project-relative target lexically: normal components only, no
/// `.git`, and a `.md` file name.
pub fn target(relative: &str) -> Result<PathBuf, AgentsError> {
    let bad = || AgentsError::UnsafeTarget(relative.to_owned());
    let mut clean = PathBuf::new();
    for part in Path::new(relative).components() {
        match part {
            Component::Normal(p) if !is_git(p) => clean.push(p),
            _ => return Err(bad()),
        }
    }
    if clean.extension().and_then(|e| e.to_str()) != Some("md") {
        return Err(bad());
    }
    Ok(clean)
}

/// Set `O_NONBLOCK`, so an open of a FIFO or device swapped in at the leaf returns at once.
#[cfg(unix)]
fn nonblocking(options: &mut OpenOptions) {
    use cap_std::fs::OpenOptionsExt;
    // `cast_signed` keeps every bit; the flag is a small positive value.
    options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits().cast_signed());
}

#[cfg(not(unix))]
fn nonblocking(_: &mut OpenOptions) {}

/// Return the permissions of the regular file `name`, read from a handle opened
/// without following a symlink and without blocking. Return `None` if the file
/// does not exist. A symlink, FIFO, socket, device or directory is an unsafe target.
/// Any other failure keeps its I/O cause.
///
/// `resolve` already rewrote every link in the path. Refusing a leaf that
/// became a link since then keeps a swapped link from lending its target's
/// mode (CWE-59, CWE-367). The cap-std handle still confines the open to the
/// project. `rel` is the path shown in an error.
///
/// The mode and the content come from separate opens of the name. A process
/// of the same user that swaps the file between them changes the mode this
/// copies. The project accepts that race for a same-user CLI.
fn source_mode(
    parent: &Dir,
    name: &std::ffi::OsStr,
    rel: &Path,
) -> Result<Option<Permissions>, AgentsError> {
    let unsafe_target = || AgentsError::UnsafeTarget(rel.display().to_string());
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    nonblocking(&mut options);
    match parent.open_with(name, &options) {
        Ok(source) => {
            let meta = source.metadata().map_err(|e| Project::map(rel, e))?;
            if meta.is_file() {
                Ok(Some(meta.permissions()))
            } else {
                Err(unsafe_target())
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => {
            // The open fails on a link, and on a socket. Any entry that is not a
            // regular file is an unsafe target. A regular file keeps its I/O cause.
            let not_regular = parent.symlink_metadata(name).is_ok_and(|m| !m.is_file());
            Err(if not_regular {
                unsafe_target()
            } else {
                Project::map(rel, e)
            })
        }
    }
}

impl Project {
    /// Open the project root.
    pub fn open(root: &Path) -> Result<Self, AgentsError> {
        let dir = Dir::open_ambient_dir(root, ambient_authority())
            .map_err(|e| AgentsError::io(".", e))?;
        Ok(Self {
            dir,
            root: root.to_owned(),
        })
    }

    /// The path to show a user for `rel`.
    #[must_use]
    pub fn display(&self, rel: &Path) -> String {
        self.root.join(rel).display().to_string()
    }

    fn map(rel: &Path, e: std::io::Error) -> AgentsError {
        if e.kind() == ErrorKind::PermissionDenied && e.to_string().contains("outside") {
            AgentsError::UnsafeTarget(rel.display().to_string())
        } else {
            AgentsError::io(rel, e)
        }
    }

    /// Resolve every symlink in `rel`, ancestors included, to a plain path inside
    /// the project, then apply the target policy to the destination: no `.git`
    /// component (any case) and a `.md` file name. A link that climbs out of the
    /// project, is absolute, or loops fails with `UnsafeTarget`.
    ///
    /// Trade-off: the policy check reads the links once, and the resolved path is
    /// then opened through the directory handle. The handle keeps every access
    /// inside the project, but a process that rewrites links in the project between
    /// the check and the open could still aim a component at `.git`. A process that
    /// can do that can write `.git` directly, so for a same-user CLI the check is a
    /// guard against mistakes, not a sandbox.
    fn resolve(&self, rel: &Path) -> Result<PathBuf, AgentsError> {
        let bad = || AgentsError::UnsafeTarget(rel.display().to_string());
        let mut pending: VecDeque<Step> = steps(rel).ok_or_else(bad)?;
        let mut done = PathBuf::new();
        let mut hops = 0;
        while let Some(step) = pending.pop_front() {
            let name = match step {
                Step::Up => {
                    if !done.pop() {
                        return Err(bad());
                    }
                    continue;
                }
                Step::Name(name) => name,
            };
            if is_git(&name) {
                return Err(bad());
            }
            let candidate = done.join(&name);
            let is_link = match self.dir.symlink_metadata(&candidate) {
                Ok(m) => m.file_type().is_symlink(),
                Err(e) if e.kind() == ErrorKind::NotFound => false,
                Err(e) => return Err(Self::map(&candidate, e)),
            };
            if !is_link {
                done = candidate;
                continue;
            }
            hops += 1;
            if hops > MAX_LINKS {
                return Err(bad());
            }
            let link = self
                .dir
                .read_link_contents(&candidate)
                .map_err(|e| Self::map(&candidate, e))?;
            for step in steps(&link).ok_or_else(bad)?.into_iter().rev() {
                pending.push_front(step);
            }
        }
        if done.extension().and_then(|e| e.to_str()) != Some("md") {
            return Err(bad());
        }
        Ok(done)
    }

    /// The bytes of the file at `rel`, or `None` if it does not exist.
    pub fn read(&self, rel: &Path) -> Result<Option<Vec<u8>>, AgentsError> {
        let rel = self.resolve(rel)?;
        match self.dir.metadata(&rel) {
            Ok(m) if !m.is_file() => {
                return Err(AgentsError::UnsafeTarget(rel.display().to_string()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Self::map(&rel, e)),
        }
        self.dir
            .read(&rel)
            .map(Some)
            .map_err(|e| Self::map(&rel, e))
    }

    /// Whether a file exists at `rel`.
    pub fn exists(&self, rel: &Path) -> Result<bool, AgentsError> {
        Ok(self.read(rel)?.is_some())
    }

    fn parent_dir(&self, rel: &Path, create: bool) -> Result<Dir, AgentsError> {
        let parent = rel.parent().filter(|p| !p.as_os_str().is_empty());
        let Some(parent) = parent else {
            return self.dir.try_clone().map_err(|e| Self::map(rel, e));
        };
        if create {
            self.dir
                .create_dir_all(parent)
                .map_err(|e| Self::map(parent, e))?;
        }
        self.dir.open_dir(parent).map_err(|e| Self::map(parent, e))
    }

    /// The bytes now at `name` in `parent`, or `None` when no entry has the name.
    ///
    /// This is the last look before a commit, so it does not resolve links
    /// again: a name that has turned into a symlink, a directory or another
    /// special file since the first check counts as changed.
    fn current(parent: &Dir, name: &OsStr, rel: &Path) -> Result<Option<Vec<u8>>, AgentsError> {
        match parent.symlink_metadata(name) {
            Ok(m) if !m.is_file() => Err(AgentsError::Conflict(rel.to_path_buf())),
            Ok(_) => parent.read(name).map(Some).map_err(|e| Self::map(rel, e)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Self::map(rel, e)),
        }
    }

    /// Replace the file at `rel` with `after`, but only if it still holds
    /// `before`. Returns whether the file changed.
    ///
    /// The new bytes go to a temporary file in the target's directory, which is
    /// synced and then renamed over the name. The target is compared with
    /// `before` once at the start and again right before the rename, so a change
    /// that another writer, such as an editor or a pre-commit hook, makes while
    /// the temporary file is written ends in [`AgentsError::Conflict`] and keeps
    /// that writer's bytes.
    ///
    /// Guarantee: this narrows the lost-update window to the gap between the
    /// last read and the rename. It is not a lock. A write that lands in that
    /// gap is still replaced. It assumes that no attacker can write to the
    /// target's directory. Such a process could swap the name in that gap, and
    /// the check only guards cooperating writers against a lost update.
    pub fn write_atomic(
        &self,
        rel: &Path,
        before: Option<&[u8]>,
        after: &[u8],
    ) -> Result<bool, AgentsError> {
        self.write_atomic_staged(rel, before, after, &mut |_| {})
    }

    /// [`Project::write_atomic`], calling `hook` at each [`Stage`], so a test can
    /// observe or change the target there. [`Stage::Filled`] comes once the
    /// temporary file holds the new bytes and before the code sets its mode.
    fn write_atomic_staged(
        &self,
        rel: &Path,
        before: Option<&[u8]>,
        after: &[u8],
        hook: &mut dyn FnMut(Stage),
    ) -> Result<bool, AgentsError> {
        if before == Some(after) {
            return Ok(false);
        }
        let rel = self.resolve(rel)?;
        if self.read(&rel)?.as_deref() != before {
            return Err(AgentsError::Conflict(rel.clone()));
        }
        let parent = self.parent_dir(&rel, true)?;
        let name = rel
            .file_name()
            .ok_or_else(|| AgentsError::UnsafeTarget(rel.display().to_string()))?;
        // The mode comes from metadata on a no-follow, non-blocking source
        // handle, not from a path stat.
        let mode = source_mode(&parent, name, &rel)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        // Private until filled, so the new bytes are never readable by others.
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let mut attempt = 0_u32;
        let (tmp_name, mut file) = loop {
            let tmp_name = format!(".slopvac-{}-{stamp}-{attempt}", std::process::id());
            match parent.open_with(&tmp_name, &options) {
                Ok(f) => break (tmp_name, f),
                Err(e) if e.kind() == ErrorKind::AlreadyExists && attempt < 100 => attempt += 1,
                Err(e) => return Err(Self::map(&rel, e)),
            }
        };
        let finish = || -> std::io::Result<()> {
            file.write_all(after)?;
            hook(Stage::Filled);
            if let Some(mode) = mode {
                file.set_permissions(mode)?;
            }
            file.sync_all()
        };
        let result = finish().map_err(|e| Self::map(&rel, e)).and_then(|()| {
            drop(file);
            hook(Stage::BeforeCommit);
            if Self::current(&parent, name, &rel)?.as_deref() != before {
                return Err(AgentsError::Conflict(rel.clone()));
            }
            parent
                .rename(&tmp_name, &parent, name)
                .map_err(|e| Self::map(&rel, e))
        });
        if result.is_err() {
            // Best effort: the error being returned matters more than a leftover temporary file.
            let _ = parent.remove_file(&tmp_name);
        }
        result?;
        Ok(true)
    }

    /// Delete the file at `rel`, but only if it still holds `expected`, and its
    /// parent directory if that is now empty.
    ///
    /// The comparison runs right before the delete, so a file that changed
    /// since the caller read it stays and the call fails with
    /// [`AgentsError::Conflict`]. The guarantee and its limits are those of
    /// [`Project::write_atomic`].
    pub fn remove_file(&self, rel: &Path, expected: &[u8]) -> Result<(), AgentsError> {
        self.remove_file_staged(rel, expected, &mut |_| {})
    }

    /// [`Project::remove_file`], calling `hook` at each [`Stage`], so a test can
    /// change the target there.
    fn remove_file_staged(
        &self,
        rel: &Path,
        expected: &[u8],
        hook: &mut dyn FnMut(Stage),
    ) -> Result<(), AgentsError> {
        let rel = self.resolve(rel)?;
        let parent = self.parent_dir(&rel, false)?;
        let name = rel
            .file_name()
            .ok_or_else(|| AgentsError::UnsafeTarget(rel.display().to_string()))?;
        hook(Stage::BeforeCommit);
        if Self::current(&parent, name, &rel)?.as_deref() != Some(expected) {
            return Err(AgentsError::Conflict(rel));
        }
        parent.remove_file(name).map_err(|e| Self::map(&rel, e))?;
        // Windows cannot delete a directory while a handle to it is open.
        drop(parent);
        if let Some(dir) = rel.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = self.dir.remove_dir(dir);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// The names in `dir`, sorted, so a leftover temporary file shows up.
    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    #[rstest]
    #[case::replaced(Some("checked"))]
    #[case::created(None)]
    fn a_write_keeps_a_change_made_after_the_check(#[case] before: Option<&str>) {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("a.md");
        if let Some(text) = before {
            std::fs::write(&target, text).unwrap();
        }
        let project = Project::open(tmp.path()).unwrap();

        let got = project.write_atomic_staged(
            Path::new("a.md"),
            before.map(str::as_bytes),
            b"ours",
            &mut |stage| {
                if stage == Stage::BeforeCommit {
                    std::fs::write(&target, "theirs").unwrap();
                }
            },
        );

        assert!(matches!(got, Err(AgentsError::Conflict(p)) if p == Path::new("a.md")));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "theirs");
        assert_eq!(names(tmp.path()), ["a.md"]);
    }

    #[test]
    fn a_delete_keeps_a_change_made_after_the_check() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("a.md");
        std::fs::write(&target, "checked").unwrap();
        let project = Project::open(tmp.path()).unwrap();

        let got = project.remove_file_staged(Path::new("a.md"), b"checked", &mut |stage| {
            if stage == Stage::BeforeCommit {
                std::fs::write(&target, "theirs").unwrap();
            }
        });

        assert!(matches!(got, Err(AgentsError::Conflict(p)) if p == Path::new("a.md")));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "theirs");
    }
}

#[cfg(all(test, unix))]
mod mode_tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::fs::PermissionsExt;

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_temporary_file_is_private_while_filled_and_the_target_keeps_its_mode() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.md");
        std::fs::write(&target, "old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        let project = Project::open(dir.path()).unwrap();
        let mut seen = Vec::new();
        let changed = project
            .write_atomic_staged(Path::new("a.md"), Some(b"old"), b"new", &mut |stage| {
                if stage != Stage::Filled {
                    return;
                }
                let temp = std::fs::read_dir(dir.path())
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .find(|p| {
                        p.file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with(".slopvac-"))
                    })
                    .unwrap();
                seen.push((std::fs::read(&temp).unwrap(), mode_of(&temp)));
            })
            .unwrap();
        assert!(changed);
        assert_eq!(seen, [(b"new".to_vec(), 0o600)]);
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(mode_of(&target), 0o644);
    }

    /// What `source_mode` decided, reduced to the data a case compares.
    fn verdict(dir: &Path, name: &str) -> &'static str {
        let handle = Dir::open_ambient_dir(dir, ambient_authority()).unwrap();
        match source_mode(&handle, OsStr::new(name), Path::new(name)) {
            Ok(Some(_)) => "mode",
            Ok(None) => "absent",
            Err(AgentsError::UnsafeTarget(_)) => "unsafe",
            Err(_) => "other",
        }
    }

    #[rstest::rstest]
    #[case::regular_file("real.md", "mode")]
    #[case::missing("gone.md", "absent")]
    #[case::symlink_to_a_file("link.md", "unsafe")]
    #[case::directory("dir.md", "unsafe")]
    fn the_source_mode_is_taken_only_from_a_regular_file(
        #[case] name: &str,
        #[case] expected: &str,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.md"), "x").unwrap();
        std::fs::create_dir(dir.path().join("dir.md")).unwrap();
        std::os::unix::fs::symlink("real.md", dir.path().join("link.md")).unwrap();
        assert_eq!(verdict(dir.path(), name), expected);
    }

    #[test]
    fn a_socket_at_the_source_is_an_unsafe_target() {
        let dir = tempfile::tempdir().unwrap();
        let _listener = std::os::unix::net::UnixListener::bind(dir.path().join("sock.md")).unwrap();
        assert_eq!(verdict(dir.path(), "sock.md"), "unsafe");
    }

    #[test]
    fn a_fifo_at_the_source_is_refused_without_blocking() {
        use rustix::fs::{CWD, FileType, Mode, mknodat};
        let dir = tempfile::tempdir().unwrap();
        mknodat(
            CWD,
            dir.path().join("pipe.md"),
            FileType::Fifo,
            Mode::RUSR | Mode::WUSR,
            0,
        )
        .unwrap();
        // Run the open on a thread and bound the wait, so a blocked open fails
        // the test instead of hanging it. A pass returns at once.
        let (tx, rx) = std::sync::mpsc::channel();
        let path = dir.path().to_owned();
        std::thread::spawn(move || {
            let handle = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
            // The receiver may have timed out and gone; the send result does not matter.
            let _ = tx.send(source_mode(
                &handle,
                OsStr::new("pipe.md"),
                Path::new("pipe.md"),
            ));
        });
        let got = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the open blocked on a FIFO");
        assert!(matches!(got, Err(AgentsError::UnsafeTarget(_))));
    }

    #[test]
    fn a_new_file_ends_private() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::open(dir.path()).unwrap();
        project.write_atomic(Path::new("n.md"), None, b"x").unwrap();
        assert_eq!(mode_of(&dir.path().join("n.md")), 0o600);
    }
}
