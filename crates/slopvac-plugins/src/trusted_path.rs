//! Finding a program only in trusted `PATH` directories (CWE-427).
//!
//! `git` and OCI credential helpers are looked up through `PATH`, which a
//! hostile checkout can influence: a relative or empty entry resolves against
//! the working directory, and an absolute entry can point into the checkout.
//! A planted program would run with the user's rights, so the lookup skips
//! such directories instead.
//!
//! A directory is searched only when it is absolute, resolves (links followed)
//! to a place outside the project, and is trusted. The program found is
//! resolved the same way and must pass the same checks, and the resolved path
//! is the one to run.
//!
//! The program, its directory and every ancestor must belong to root or the
//! current user, and must not be writable by others. One exception: a sticky,
//! root-owned ancestor such as `/tmp` may be, because others cannot replace
//! what they do not own.
//!
//! Writable by group is refused, except for one group: `admin`, looked up by
//! name (macOS: gid 80), when the current process belongs to it. An
//! administrator can already become root, so the group adds no write that the
//! user could not make anyway. This is how `/Applications` is set up, where
//! Docker Desktop keeps its credential helper, and how Homebrew's
//! `/usr/local/bin` is often set up. Any other group is refused, even when the
//! user looks like its only member: `/etc/group` does not list accounts that a
//! directory service holds, so absence from it proves nothing.

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
    find_in(program, &path, &project, &SystemHost)
}

/// What a directory entry says about who may change it.
#[derive(Clone, Copy)]
struct Stat {
    uid: u32,
    gid: u32,
    mode: u32,
}

/// The facts about the machine the trust check reads, so tests can stand in
/// for a root-owned directory or another user. [`SystemHost`] is the real one.
trait Host {
    /// Owner, group and mode of `path`, links followed.
    fn stat(&self, path: &Path) -> Option<Stat>;
    /// The current user.
    fn uid(&self) -> u32;
    /// Whether the current user belongs to group `gid`.
    fn in_group(&self, gid: u32) -> bool;
    /// The group id named `admin`.
    fn admin_gid(&self) -> Option<u32>;
}

fn find_in(program: &str, path: &OsStr, project: &Path, host: &dyn Host) -> Option<PathBuf> {
    let trusted: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .filter_map(|dir| std::fs::canonicalize(dir).ok())
        .filter(|dir| !dir.starts_with(project) && trusted_path(dir, host))
        .collect();
    // An empty search path would otherwise mean the working directory.
    if trusted.is_empty() {
        return None;
    }
    let found = find_in_path(program, Some(&std::env::join_paths(trusted).ok()?))?;
    // Run the program the checks saw, not a link that could be repointed.
    let real = std::fs::canonicalize(&found).ok()?;
    let regular = std::fs::metadata(&real).is_ok_and(|m| m.is_file());
    (regular && !real.starts_with(project) && trusted_path(&real, host)).then_some(real)
}

/// Whether nobody but root or the current user can change what `path` names
/// (see the module documentation for the rule). `path` is canonical, so no
/// link is left to repoint.
///
/// Windows has no such mode bits and is not a supported platform for 3.0.
fn trusted_path(path: &Path, host: &dyn Host) -> bool {
    let me = host.uid();
    path.ancestors().all(|step| {
        let Some(meta) = host.stat(step) else {
            return false;
        };
        let owner_ok = meta.uid == 0 || meta.uid == me;
        let others = meta.mode & 0o002 != 0;
        let group = meta.mode & 0o020 != 0;
        let sticky_root = meta.mode & 0o1000 != 0 && meta.uid == 0;
        let tolerated_open = step != path && sticky_root;
        owner_ok && (tolerated_open || (!others && (!group || group_is_closed(&meta, host))))
    })
}

/// Whether a group-writable entry grants a write to nobody but administrators
/// that the current user belongs to.
fn group_is_closed(meta: &Stat, host: &dyn Host) -> bool {
    host.admin_gid() == Some(meta.gid) && host.in_group(meta.gid)
}

/// The machine this process runs on.
struct SystemHost;

#[cfg(unix)]
impl Host for SystemHost {
    fn stat(&self, path: &Path) -> Option<Stat> {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).ok()?;
        Some(Stat {
            uid: meta.uid(),
            gid: meta.gid(),
            mode: meta.mode(),
        })
    }

    fn uid(&self) -> u32 {
        rustix::process::getuid().as_raw()
    }

    fn in_group(&self, gid: u32) -> bool {
        rustix::process::getgid().as_raw() == gid
            || rustix::process::getgroups().is_ok_and(|all| all.iter().any(|g| g.as_raw() == gid))
    }

    fn admin_gid(&self) -> Option<u32> {
        group_gid(&std::fs::read_to_string("/etc/group").ok()?, "admin")
    }
}

#[cfg(not(unix))]
impl Host for SystemHost {
    fn stat(&self, _: &Path) -> Option<Stat> {
        Some(Stat {
            uid: 0,
            gid: 0,
            mode: 0o755,
        })
    }
    fn uid(&self) -> u32 {
        0
    }
    fn in_group(&self, _: u32) -> bool {
        false
    }
    fn admin_gid(&self) -> Option<u32> {
        None
    }
}

/// The id of group `name` in the text of `/etc/group`.
fn group_gid(groups: &str, name: &str) -> Option<u32> {
    groups.lines().find_map(|line| {
        let mut f = line.split(':');
        if f.next()? != name {
            return None;
        }
        f.nth(1)?.parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(path: &OsStr, project: &Path) -> Option<PathBuf> {
        find_in("tool", path, project, &SystemHost)
    }

    /// The real machine, except for the entries named in `over` and for the
    /// user and groups, which the test sets. Tests cannot create a
    /// root-owned directory, so they claim one.
    #[cfg(unix)]
    struct Fake {
        over: Vec<(PathBuf, Stat)>,
        uid: u32,
        groups: Vec<u32>,
        admin: Option<u32>,
    }

    #[cfg(unix)]
    impl Fake {
        fn me() -> Self {
            Self {
                over: Vec::new(),
                uid: SystemHost.uid(),
                groups: Vec::new(),
                admin: Some(80),
            }
        }

        fn claim(mut self, path: &Path, uid: u32, gid: u32, mode: u32) -> Self {
            self.over.push((path.to_owned(), Stat { uid, gid, mode }));
            self
        }
    }

    #[cfg(unix)]
    impl Host for Fake {
        fn stat(&self, path: &Path) -> Option<Stat> {
            match self.over.iter().find(|(p, _)| p == path) {
                Some((_, stat)) => Some(*stat),
                None => SystemHost.stat(path),
            }
        }
        fn uid(&self) -> u32 {
            self.uid
        }
        fn in_group(&self, gid: u32) -> bool {
            self.groups.contains(&gid)
        }
        fn admin_gid(&self) -> Option<u32> {
            self.admin
        }
    }

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
        assert_eq!(find(&relative, &l.project), None);
    }

    #[cfg(unix)]
    #[test]
    fn an_absolute_entry_after_a_relative_one_still_finds_it() {
        let l = layout();
        let path = path_of(&[Path::new("."), &l.trusted]);
        assert_eq!(find(&path, &l.project), Some(l.trusted.join("tool")));
    }

    #[cfg(unix)]
    #[test]
    fn a_program_inside_the_project_is_ignored_even_first_on_the_path() {
        let l = layout();
        let path = path_of(&[&l.local, &l.trusted]);
        assert_eq!(find(&path, &l.project), Some(l.trusted.join("tool")));
        assert_eq!(find(&path_of(&[&l.local]), &l.project), None);
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
        assert_eq!(find(&path, &l.project), None);
        // A link to the project directory itself is resolved before the check.
        let alias = l.trusted.parent().unwrap().join("alias");
        std::os::unix::fs::symlink(&l.local, &alias).unwrap();
        assert_eq!(find(&path_of(&[&alias]), &l.project), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_file_in_a_writable_directory_is_refused() {
        let l = layout();
        // The link sits in a protected directory; the file it names does not.
        let linked = l.trusted.parent().unwrap().join("linked-open");
        std::fs::create_dir(&linked).unwrap();
        std::os::unix::fs::symlink(l.open.join("tool"), linked.join("tool")).unwrap();
        assert_eq!(find(&path_of(&[&linked]), &l.project), None);
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
        assert_eq!(find(&path_of(&[&inner]), &l.project), None);
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
            find(&path_of(&[&bin]), &l.project),
            Some(real_dir.join("tool-2"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_others_can_write_is_not_trusted() {
        let l = layout();
        assert_eq!(find(&path_of(&[&l.open]), &l.project), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_program_owned_by_another_user_is_refused() {
        // The tests run as the owner of every file here. Standing in for a
        // different user makes the same files belong to someone else.
        let l = layout();
        let path = path_of(&[&l.trusted]);
        let found = |host: &Fake| find_in("tool", &path, &l.project, host);
        assert_eq!(found(&Fake::me()), Some(l.trusted.join("tool")));
        let other = Fake {
            uid: SystemHost.uid() + 1,
            ..Fake::me()
        };
        assert_eq!(found(&other), None);
    }

    /// Docker Desktop's layout: a link in a trusted `bin` to a program under a
    /// root-owned directory (claimed here) that group `admin` (gid 80) can write.
    #[cfg(unix)]
    fn desktop_layout() -> (Layout, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let l = layout();
        let base = l.trusted.parent().unwrap().to_owned();
        let apps = base.join("apps");
        std::fs::create_dir(&apps).unwrap();
        std::fs::write(apps.join("desktop-tool"), b"").unwrap();
        std::fs::set_permissions(&apps, std::fs::Permissions::from_mode(0o775)).unwrap();
        let bin = base.join("usr-bin");
        std::fs::create_dir(&bin).unwrap();
        std::os::unix::fs::symlink(apps.join("desktop-tool"), bin.join("tool")).unwrap();
        (l, bin, apps)
    }

    /// `owner`: 0 for root, otherwise another user; the current user stays
    /// the owner when `None`.
    #[cfg(unix)]
    fn desktop_host(apps: &Path, owner: Option<u32>, gid: u32, mode: u32) -> Fake {
        Fake {
            groups: vec![80],
            ..Fake::me()
        }
        .claim(apps, owner.unwrap_or(SystemHost.uid()), gid, mode)
    }

    #[cfg(unix)]
    #[rstest::rstest]
    #[case::owned_by_root(Some(0))]
    #[case::owned_by_the_current_user(None)]
    fn an_admin_group_directory_is_trusted_for_a_member(#[case] owner: Option<u32>) {
        let (l, bin, apps) = desktop_layout();
        let host = desktop_host(&apps, owner, 80, 0o775);
        assert_eq!(
            find_in("tool", &path_of(&[&bin]), &l.project, &host),
            Some(apps.join("desktop-tool"))
        );
    }

    #[cfg(unix)]
    #[rstest::rstest]
    #[case::not_a_member(vec![], Some(80), None, 80)]
    #[case::group_is_not_admin(vec![80], Some(81), None, 80)]
    #[case::admin_group_missing(vec![80], None, None, 80)]
    #[case::owned_by_another_user(vec![80], Some(80), Some(1), 80)]
    fn the_admin_exception_needs_membership_the_admin_group_and_a_trusted_owner(
        #[case] groups: Vec<u32>,
        #[case] admin: Option<u32>,
        #[case] foreign_owner: Option<u32>,
        #[case] gid: u32,
    ) {
        let (l, bin, apps) = desktop_layout();
        // Any non-zero id here differs from the current user.
        let owner = foreign_owner.map_or(SystemHost.uid(), |_| SystemHost.uid() + 1);
        let host = Fake {
            groups,
            admin,
            ..Fake::me()
        }
        .claim(&apps, owner, gid, 0o775);
        assert_eq!(find_in("tool", &path_of(&[&bin]), &l.project, &host), None);
    }

    #[cfg(unix)]
    #[rstest::rstest]
    #[case::the_only_listed_member(vec![5000])]
    #[case::not_in_the_group(vec![])]
    fn a_group_writable_directory_of_another_group_is_refused(#[case] groups: Vec<u32>) {
        // Group 5000 stands for any group but admin, here with the user as its
        // only listed member: nothing proves no one else is in it.
        let l = layout();
        let host = Fake {
            groups,
            ..Fake::me()
        }
        .claim(&l.trusted, SystemHost.uid(), 5000, 0o775);
        assert_eq!(
            find_in("tool", &path_of(&[&l.trusted]), &l.project, &host),
            None
        );
    }

    #[cfg(unix)]
    #[rstest::rstest]
    #[case::owned_by_root(Some(0))]
    #[case::owned_by_the_current_user(None)]
    fn a_world_writable_directory_is_refused_even_in_the_admin_group(#[case] owner: Option<u32>) {
        let (l, bin, apps) = desktop_layout();
        let host = desktop_host(&apps, owner, 80, 0o777);
        assert_eq!(find_in("tool", &path_of(&[&bin]), &l.project, &host), None);
    }

    const GROUPS: &str = "root:x:0:\nadmin:*:80:root,alice\nalice:x:1000:\nteam:x:1001:alice,bob\n";

    #[rstest::rstest]
    #[case::finds_it("admin", Some(80))]
    #[case::unknown_name("nobody", None)]
    #[case::not_a_prefix("adm", None)]
    fn groups_are_looked_up_by_name(#[case] name: &str, #[case] gid: Option<u32>) {
        assert_eq!(group_gid(GROUPS, name), gid);
    }
}
