//! Finding a program only in trusted `PATH` directories (CWE-427).
//!
//! `git` and OCI credential helpers are looked up through `PATH`, which a
//! hostile checkout can influence: a relative or empty entry resolves against
//! the working directory, and an absolute entry can point into the checkout.
//! A planted program would run with the user's rights, so the lookup skips
//! such directories instead.
//!
//! A directory is searched only when it is absolute, resolves (links followed)
//! to a place outside the project, and is trusted: it and every ancestor
//! belong to root or the current user, and are closed to group and others. A
//! sticky, root-owned ancestor such as `/tmp` is allowed. The program found is
//! resolved the same way and must pass the same checks, and the resolved path
//! is the one to run.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::oci::find_in_path;

/// `program` from the first trusted directory of `search` (default: the
/// `PATH` variable), as a link-resolved path. `project_root` anchors the check
/// that the program is not inside the project.
pub(crate) fn find_trusted(
    program: &str,
    search: Option<&OsStr>,
    project_root: &Path,
) -> Option<PathBuf> {
    let project = std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_owned());
    let path = match search {
        Some(path) => path.to_owned(),
        None => std::env::var_os("PATH")?,
    };
    find_in(program, &path, &project)
}

fn find_in(program: &str, path: &OsStr, project: &Path) -> Option<PathBuf> {
    find_as(program, path, project, current_uid())
}

/// [`find_in`] for a given user id, so tests can stand in for another user.
fn find_as(program: &str, path: &OsStr, project: &Path, me: u32) -> Option<PathBuf> {
    let trusted: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .filter_map(|dir| std::fs::canonicalize(dir).ok())
        .filter(|dir| !dir.starts_with(project) && trusted_path(dir, me))
        .collect();
    // An empty search path would otherwise mean the working directory.
    if trusted.is_empty() {
        return None;
    }
    let found = find_in_path(program, Some(&std::env::join_paths(trusted).ok()?))?;
    // Run the program the checks saw, not a link that could be repointed.
    let real = std::fs::canonicalize(&found).ok()?;
    let regular = std::fs::metadata(&real).is_ok_and(|m| m.is_file());
    (regular && !real.starts_with(project) && trusted_path(&real, me)).then_some(real)
}

#[cfg(unix)]
fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// Whether nobody but root or `me` can change what `path` names: `path` and
/// every ancestor belong to root or `me`. `path` itself is not writable by
/// group or others. An ancestor may be, only when it is sticky and owned by
/// root (`/tmp`), since then others cannot replace what they do not own.
/// `path` is canonical, so no link is left to repoint.
///
/// Windows has no such mode bits and is not a supported platform for 3.0.
fn trusted_path(path: &Path, me: u32) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        path.ancestors().all(|step| {
            let Ok(meta) = std::fs::metadata(step) else {
                return false;
            };
            let owner_ok = meta.uid() == 0 || meta.uid() == me;
            let open = meta.mode() & 0o022 != 0;
            let sticky_root = meta.mode() & 0o1000 != 0 && meta.uid() == 0;
            owner_ok && (!open || (step != path && sticky_root))
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (path, me);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch tree: `project/bin`, a private `trusted` directory and an
    /// `open` directory that anyone can write to, each holding a `tool` file.
    #[cfg(unix)]
    struct Layout {
        _dir: tempfile::TempDir,
        project: PathBuf,
        local: PathBuf,
        trusted: PathBuf,
        open: PathBuf,
    }

    #[cfg(unix)]
    fn layout() -> Layout {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let make = |rel: &str, mode: u32| {
            let path = base.join(rel);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("tool"), b"").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            path
        };
        let project = base.join("project");
        let local = make("project/bin", 0o755);
        let trusted = make("trusted", 0o755);
        let open = make("open", 0o777);
        Layout {
            _dir: dir,
            project,
            local,
            trusted,
            open,
        }
    }

    #[cfg(unix)]
    fn path_of(dirs: &[&Path]) -> std::ffi::OsString {
        std::env::join_paths(dirs).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn relative_path_entries_never_find_the_program() {
        let l = layout();
        // `trusted` is reachable only through a relative entry.
        let relative = path_of(&[Path::new("."), Path::new("sub/bin")]);
        assert_eq!(find_in("tool", &relative, &l.project), None);
    }

    #[cfg(unix)]
    #[test]
    fn an_absolute_entry_after_a_relative_one_still_finds_it() {
        let l = layout();
        let path = path_of(&[Path::new("."), &l.trusted]);
        assert_eq!(
            find_in("tool", &path, &l.project),
            Some(l.trusted.join("tool"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_program_inside_the_project_is_ignored_even_first_on_the_path() {
        let l = layout();
        let path = path_of(&[&l.local, &l.trusted]);
        assert_eq!(
            find_in("tool", &path, &l.project),
            Some(l.trusted.join("tool"))
        );
        assert_eq!(find_in("tool", &path_of(&[&l.local]), &l.project), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_into_the_project_does_not_hide_a_planted_program() {
        let l = layout();
        // `/usr/bin`-like directory whose `tool` is a link to the project's file.
        let linked = l.trusted.parent().unwrap().join("linked");
        std::fs::create_dir(&linked).unwrap();
        std::os::unix::fs::symlink(l.local.join("tool"), linked.join("tool")).unwrap();
        let path = path_of(&[&linked]);
        assert_eq!(find_in("tool", &path, &l.project), None);
        // A link to the project directory itself is resolved before the check.
        let alias = l.trusted.parent().unwrap().join("alias");
        std::os::unix::fs::symlink(&l.local, &alias).unwrap();
        assert_eq!(find_in("tool", &path_of(&[&alias]), &l.project), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_file_in_a_writable_directory_is_refused() {
        let l = layout();
        // The link sits in a protected directory; the file it names does not.
        let linked = l.trusted.parent().unwrap().join("linked-open");
        std::fs::create_dir(&linked).unwrap();
        std::os::unix::fs::symlink(l.open.join("tool"), linked.join("tool")).unwrap();
        assert_eq!(find_in("tool", &path_of(&[&linked]), &l.project), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_protected_directory_under_a_writable_ancestor_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let l = layout();
        let inner = l.open.join("inner");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(inner.join("tool"), b"").unwrap();
        std::fs::set_permissions(&inner, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_in("tool", &path_of(&[&inner]), &l.project), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_program_owned_by_another_user_is_refused() {
        // The tests run as the owner of every file here. Standing in for a
        // different user makes the same files belong to someone else.
        let l = layout();
        let path = path_of(&[&l.trusted]);
        let me = current_uid();
        assert_eq!(
            find_as("tool", &path, &l.project, me),
            Some(l.trusted.join("tool"))
        );
        assert_eq!(find_as("tool", &path, &l.project, me + 1), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_resolved_path_is_what_the_lookup_returns() {
        let l = layout();
        let real_dir = l.trusted.parent().unwrap().join("real");
        std::fs::create_dir(&real_dir).unwrap();
        std::fs::write(real_dir.join("tool-2"), b"").unwrap();
        let bin = l.trusted.parent().unwrap().join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::os::unix::fs::symlink(real_dir.join("tool-2"), bin.join("tool")).unwrap();
        assert_eq!(
            find_in("tool", &path_of(&[&bin]), &l.project),
            Some(real_dir.join("tool-2"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_others_can_write_is_not_trusted() {
        let l = layout();
        assert_eq!(find_in("tool", &path_of(&[&l.open]), &l.project), None);
    }
}
