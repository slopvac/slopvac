//! Apply and restore in a checkout whose author controls every file: links, swapped files,
//! edits, and forged, extended or replayed backups.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rstest::rstest;
use slopvac_migrate::{
    BACKUP_DIR, Change, ChangeKind, Error, NullLinter, Options, Plan, Point, RenameTable, Restored,
    apply, apply_hooked, command, plan, restore, restore_hooked, run,
};

const STEERING: &str = "<!-- slopvac:begin -->\nnew\n<!-- slopvac:end -->\n";
const OLD: &str = "profile = \"strict\"\n";
const NEW: &str = "profile = \"strict\"\nschema_version = 3\n";
const EDITED: &str = "profile = \"normal\"\n";
const DOC_OLD: &str = "<!-- slopvac-allow: rule=orwell.not-un reason=x -->\n";

/// A project next to a file outside it, and a per-user cache directory.
struct Setup {
    outer: tempfile::TempDir,
    cache: tempfile::TempDir,
}

impl Setup {
    fn new(files: &[(&str, &str)]) -> Self {
        let outer = tempfile::tempdir().unwrap();
        fs::create_dir(outer.path().join("proj")).unwrap();
        let s = Self {
            outer,
            cache: tempfile::tempdir().unwrap(),
        };
        for (name, text) in files {
            s.write(name, text);
        }
        s
    }

    fn root(&self) -> PathBuf {
        self.outer.path().join("proj")
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root().join(name)
    }

    fn outside(&self) -> PathBuf {
        self.outer.path().join("outside.toml")
    }

    fn write(&self, name: &str, text: &str) {
        let p = self.path(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.path(name)).unwrap()
    }

    fn mode(&self, name: &str) -> u32 {
        fs::symlink_metadata(self.path(name)).unwrap().mode() & 0o7777
    }

    fn plan(&self) -> Plan {
        plan(
            &self.root(),
            &RenameTable::bundled(),
            Options::default(),
            STEERING,
        )
        .unwrap()
    }

    fn apply(&self, p: &Plan) -> Result<(), Error> {
        apply(&self.root(), p, self.cache.path())
    }

    fn apply_hooked(&self, p: &Plan, hook: &mut dyn FnMut(Point<'_>)) -> Result<(), Error> {
        self.apply_failing(p, &mut |point| {
            hook(point);
            Ok(())
        })
    }

    fn apply_failing(
        &self,
        p: &Plan,
        hook: &mut dyn FnMut(Point<'_>) -> std::io::Result<()>,
    ) -> Result<(), Error> {
        apply_hooked(&self.root(), p, self.cache.path(), hook)
    }

    fn restore(&self, force: bool) -> Result<Restored, Error> {
        restore(&self.root(), self.cache.path(), force)
    }

    fn restore_hooked(
        &self,
        force: bool,
        hook: &mut dyn FnMut(Point<'_>),
    ) -> Result<Restored, Error> {
        restore_hooked(&self.root(), self.cache.path(), force, &mut |point| {
            hook(point);
            Ok(())
        })
    }

    fn backup_exists(&self) -> bool {
        fs::symlink_metadata(self.path(BACKUP_DIR)).is_ok()
    }
}

/// What a hostile checkout puts at a target.
#[derive(Debug, Clone, Copy)]
enum Swap {
    /// Another name for the outside file, with the same bytes.
    Hardlink,
    /// A symbolic link to the outside file.
    Symlink,
    /// Different bytes in place.
    Bytes,
}

impl Swap {
    fn run(self, s: &Setup, name: &str) {
        let target = s.path(name);
        let bytes = fs::read(&target).unwrap();
        match self {
            Self::Bytes => fs::write(&target, EDITED).unwrap(),
            Self::Hardlink | Self::Symlink => {
                fs::write(s.outside(), &bytes).unwrap();
                fs::remove_file(&target).unwrap();
                if matches!(self, Self::Hardlink) {
                    fs::hard_link(s.outside(), &target).unwrap();
                } else {
                    std::os::unix::fs::symlink(s.outside(), &target).unwrap();
                }
            }
        }
    }

    fn refused<T>(self, result: &Result<T, Error>) -> bool {
        match self {
            Self::Bytes => matches!(result, Err(Error::Changed(_))),
            Self::Hardlink | Self::Symlink => matches!(result, Err(Error::Unsafe(_))),
        }
    }

    /// The object the swap left at `name` is still there, and the outside file holds `bytes`.
    fn assert_untouched(self, s: &Setup, name: &str, bytes: &str) {
        let target = s.path(name);
        match self {
            Self::Bytes => assert_eq!(s.read(name), EDITED),
            Self::Hardlink => {
                assert_eq!(fs::read_to_string(s.outside()).unwrap(), bytes);
                assert_eq!(
                    fs::symlink_metadata(&target).unwrap().ino(),
                    fs::metadata(s.outside()).unwrap().ino()
                );
            }
            Self::Symlink => {
                assert_eq!(fs::read_to_string(s.outside()).unwrap(), bytes);
                assert_eq!(fs::read_link(&target).unwrap(), s.outside());
            }
        }
    }
}

/// Run `swap` on `name` the first time the hook reaches its final check.
fn once<'a>(s: &'a Setup, swap: Swap, name: &'static str) -> impl FnMut(Point<'_>) + 'a {
    let mut done = false;
    move |point| {
        if !done && point == Point::Check(Path::new(name)) {
            done = true;
            swap.run(s, name);
        }
    }
}

/// The one holding directory a change has open in `dir`.
fn holding(dir: &Path) -> PathBuf {
    let found: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".slopvac-hold-")
        })
        .collect();
    assert_eq!(found.len(), 1, "{found:?}");
    found.into_iter().next().unwrap()
}

/// Put a hard link to the file outside the project at `path`, in place of what is there.
fn link_outside(s: &Setup, path: &Path) {
    fs::remove_file(path).unwrap();
    fs::hard_link(s.outside(), path).unwrap();
}

fn outside_links(s: &Setup) -> u64 {
    fs::metadata(s.outside()).unwrap().nlink()
}

#[rstest]
#[case::hardlink(Swap::Hardlink)]
#[case::symlink(Swap::Symlink)]
#[case::bytes(Swap::Bytes)]
fn apply_refuses_a_target_changed_after_the_plan(#[case] swap: Swap) {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    let p = s.plan();
    swap.run(&s, "slopvac.toml");
    assert!(swap.refused(&s.apply(&p)));
    swap.assert_untouched(&s, "slopvac.toml", OLD);
    assert!(!s.backup_exists());
}

#[rstest]
#[case::hardlink(Swap::Hardlink)]
#[case::symlink(Swap::Symlink)]
#[case::bytes(Swap::Bytes)]
fn apply_refuses_a_target_swapped_at_the_moment_of_the_write(#[case] swap: Swap) {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    let p = s.plan();
    let result = s.apply_hooked(&p, &mut once(&s, swap, "slopvac.toml"));
    assert!(swap.refused(&result));
    swap.assert_untouched(&s, "slopvac.toml", OLD);
    assert!(
        !s.backup_exists(),
        "nothing was written, so the backup is gone"
    );
    assert!(matches!(s.restore(false), Err(Error::Unsafe(_))));
}

#[test]
fn apply_refusal_on_a_later_entry_touches_nothing() {
    let s = Setup::new(&[("slopvac.toml", OLD), ("a.md", DOC_OLD), ("b.md", DOC_OLD)]);
    let p = s.plan();
    assert_eq!(p.changes.last().unwrap().path, Path::new("b.md"));
    s.write("b.md", EDITED);
    assert!(matches!(s.apply(&p), Err(Error::Changed(_))));
    assert_eq!(s.read("slopvac.toml"), OLD);
    assert_eq!(s.read("a.md"), DOC_OLD);
    assert!(!s.backup_exists());
}

#[test]
fn apply_refuses_a_target_created_after_the_plan() {
    let s = Setup::new(&[(".slopvac.toml", OLD)]);
    let p = s.plan();
    s.write("slopvac.toml", NEW);
    assert!(matches!(s.apply(&p), Err(Error::Changed(_))));
    assert_eq!(s.read("slopvac.toml"), NEW);
    assert_eq!(s.read(".slopvac.toml"), OLD);
    assert!(!s.backup_exists());
}

#[test]
fn restore_never_deletes_a_file_that_apply_did_not_create() {
    // The root config is rewritten first. Then, at the moment apply would create
    // sub/slopvac.toml, a file with exactly the bytes apply would write appears there.
    let s = Setup::new(&[("slopvac.toml", OLD), ("sub/.slopvac.toml", OLD)]);
    let p = s.plan();
    let mut hook = |point: Point<'_>| {
        if point == Point::Check(Path::new("sub/slopvac.toml")) {
            fs::write(s.path("sub/slopvac.toml"), NEW).unwrap();
        }
    };
    assert!(matches!(
        s.apply_hooked(&p, &mut hook),
        Err(Error::Changed(_))
    ));
    assert_eq!(s.read("slopvac.toml"), NEW);
    let done = s.restore(false).unwrap();
    assert_eq!(s.read("slopvac.toml"), OLD);
    assert_eq!(s.read("sub/slopvac.toml"), NEW);
    assert_eq!(s.read("sub/.slopvac.toml"), OLD);
    assert_eq!(done.kept, vec![PathBuf::from("sub/slopvac.toml")]);
}

#[rstest]
#[case::hardlink(Swap::Hardlink)]
#[case::symlink(Swap::Symlink)]
fn restore_refuses_a_target_changed_after_the_write(#[case] swap: Swap) {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    swap.run(&s, "slopvac.toml");
    assert!(swap.refused(&s.restore(true)));
    swap.assert_untouched(&s, "slopvac.toml", NEW);
}

#[rstest]
#[case::hardlink(Swap::Hardlink)]
#[case::symlink(Swap::Symlink)]
#[case::bytes(Swap::Bytes)]
fn restore_refuses_a_target_swapped_at_the_moment_of_the_write(#[case] swap: Swap) {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    let result = s.restore_hooked(false, &mut once(&s, swap, "slopvac.toml"));
    assert!(swap.refused(&result));
    swap.assert_untouched(&s, "slopvac.toml", NEW);
    assert!(s.backup_exists());
}

#[test]
fn restore_refuses_a_file_edited_after_the_write_and_touches_nothing() {
    let s = Setup::new(&[("slopvac.toml", OLD), ("a.md", DOC_OLD), ("b.md", DOC_OLD)]);
    s.apply(&s.plan()).unwrap();
    let doc_new = s.read("a.md");
    s.write("b.md", EDITED);
    assert!(matches!(s.restore(false), Err(Error::Changed(_))));
    assert_eq!(s.read("slopvac.toml"), NEW);
    assert_eq!(s.read("a.md"), doc_new);
    assert_eq!(s.read("b.md"), EDITED);
    assert!(s.backup_exists());
}

#[test]
fn restore_with_force_puts_back_a_file_edited_after_the_write() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    s.write("slopvac.toml", EDITED);
    let done = s.restore(true).unwrap();
    assert_eq!((done.restored, done.kept.len()), (1, 0));
    assert_eq!(s.read("slopvac.toml"), OLD);
    assert!(!s.backup_exists());
}

#[test]
fn restore_keeps_a_created_file_that_was_edited() {
    let s = Setup::new(&[(".slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    s.write("slopvac.toml", EDITED);
    let done = s.restore(true).unwrap();
    assert_eq!(done.kept, vec![PathBuf::from("slopvac.toml")]);
    assert_eq!(s.read("slopvac.toml"), EDITED);
    assert_eq!(s.read(".slopvac.toml"), OLD);
}

#[test]
fn restore_counts_a_file_already_back_to_its_original() {
    let s = Setup::new(&[("slopvac.toml", OLD), ("a.md", DOC_OLD)]);
    s.apply(&s.plan()).unwrap();
    s.write("slopvac.toml", OLD);
    let done = s.restore(false).unwrap();
    assert_eq!(done.restored, 2);
    assert_eq!(s.read("a.md"), DOC_OLD);
}

#[rstest]
#[case::replaced_file("slopvac.toml", "slopvac.toml")]
#[case::deleted_file(".slopvac.toml", ".slopvac.toml")]
fn restore_with_force_keeps_an_unrelated_file_at_a_migrated_path(
    #[case] original: &str,
    #[case] target: &str,
) {
    let s = Setup::new(&[(original, OLD)]);
    s.apply(&s.plan()).unwrap();
    // An unrelated, singly linked file takes the path the migration wrote or deleted.
    s.write("swap.tmp", EDITED);
    fs::rename(s.path("swap.tmp"), s.path(target)).unwrap();
    let inode = fs::metadata(s.path(target)).unwrap().ino();
    let done = s.restore(true).unwrap();
    assert_eq!(done.kept, vec![PathBuf::from(target)]);
    assert_eq!(s.read(target), EDITED);
    assert_eq!(fs::metadata(s.path(target)).unwrap().ino(), inode);
}

#[derive(Debug, Clone, Copy)]
enum Forgery {
    /// The checkout ships a backup directory of its own.
    PreSeeded,
    /// A real backup's manifest gains an entry.
    ManifestEdited,
    /// A real backup gains a file.
    ExtraFile,
    /// A real backup copy changes.
    CopyEdited,
}

#[rstest]
#[case::pre_seeded(Forgery::PreSeeded)]
#[case::manifest_edited(Forgery::ManifestEdited)]
#[case::extra_file(Forgery::ExtraFile)]
#[case::copy_edited(Forgery::CopyEdited)]
fn restore_rejects_a_backup_it_did_not_write(#[case] forgery: Forgery) {
    let s = Setup::new(&[("slopvac.toml", OLD), ("README.md", "keep\n")]);
    let backup = s.path(BACKUP_DIR);
    if matches!(forgery, Forgery::PreSeeded) {
        fs::create_dir(&backup).unwrap();
        fs::write(backup.join("manifest.txt"), "0 README.md\n").unwrap();
        fs::write(backup.join("0"), "other\n").unwrap();
    } else {
        s.apply(&s.plan()).unwrap();
    }
    match forgery {
        Forgery::PreSeeded => {}
        Forgery::ManifestEdited => {
            let mut text = fs::read_to_string(backup.join("manifest.txt")).unwrap();
            text.push_str("1 README.md\n");
            fs::write(backup.join("manifest.txt"), text).unwrap();
        }
        Forgery::ExtraFile => fs::write(backup.join("1"), "other\n").unwrap(),
        Forgery::CopyEdited => fs::write(backup.join("0"), EDITED).unwrap(),
    }
    assert!(matches!(s.restore(true), Err(Error::Unsafe(_))));
    assert_eq!(s.read("README.md"), "keep\n");
    let config = if matches!(forgery, Forgery::PreSeeded) {
        OLD
    } else {
        NEW
    };
    assert_eq!(s.read("slopvac.toml"), config);
}

/// Copy the files directly below `from` into a new directory `to`.
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

#[test]
fn restore_rejects_a_checkout_replaced_at_the_same_path() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    // A new directory takes the project's path, and the authentic backup and migrated file
    // move into it with their inodes, so only the root directory differs.
    let moved = s.outer.path().join("moved");
    fs::rename(s.root(), &moved).unwrap();
    fs::create_dir(s.root()).unwrap();
    for name in ["slopvac.toml", BACKUP_DIR] {
        fs::rename(moved.join(name), s.path(name)).unwrap();
    }
    assert!(matches!(s.restore(false), Err(Error::Unsafe(_))));
    assert_eq!(s.read("slopvac.toml"), NEW);
    assert!(s.backup_exists());
}

#[test]
fn restore_rejects_a_replayed_older_backup() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    let old_backup = s.outer.path().join("old-backup");
    copy_dir(&s.path(BACKUP_DIR), &old_backup);
    s.restore(false).unwrap();
    s.apply(&s.plan()).unwrap();
    // The authentic backup directory stays, with its inode; only its files become the older
    // backup's. The copies hold the same original bytes, so only the instance differs.
    let backup = s.path(BACKUP_DIR);
    let inode = fs::metadata(&backup).unwrap().ino();
    for entry in fs::read_dir(&backup).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    for entry in fs::read_dir(&old_backup).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), backup.join(entry.file_name())).unwrap();
    }
    assert_eq!(fs::metadata(&backup).unwrap().ino(), inode);
    assert!(matches!(s.restore(true), Err(Error::Unsafe(_))));
    assert_eq!(s.read("slopvac.toml"), NEW);
}

#[test]
fn backups_are_owner_only() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    fs::set_permissions(s.path("slopvac.toml"), fs::Permissions::from_mode(0o644)).unwrap();
    s.apply(&s.plan()).unwrap();
    assert_eq!(s.mode(BACKUP_DIR), 0o700);
    for entry in fs::read_dir(s.path(BACKUP_DIR)).unwrap() {
        let mode = entry.unwrap().metadata().unwrap().mode() & 0o7777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn files_keep_their_mode_through_apply_and_restore() {
    let s = Setup::new(&[(".slopvac.toml", OLD), ("a.md", DOC_OLD)]);
    for name in [".slopvac.toml", "a.md"] {
        fs::set_permissions(s.path(name), fs::Permissions::from_mode(0o640)).unwrap();
    }
    s.apply(&s.plan()).unwrap();
    assert_eq!(s.mode("a.md"), 0o640);
    s.restore(false).unwrap();
    assert_eq!(s.mode(".slopvac.toml"), 0o640);
    assert_eq!(s.mode("a.md"), 0o640);
}

fn write_change(path: PathBuf, new: &str) -> Change {
    Change {
        path,
        kind: ChangeKind::Write,
        old: None,
        new: Some(new.to_owned()),
    }
}

#[rstest]
#[case::not_utf8(vec![PathBuf::from(std::ffi::OsStr::from_bytes(b"bad\xff.toml"))])]
#[case::duplicate(vec![PathBuf::from("new.toml"), PathBuf::from("new.toml")])]
#[case::duplicate_after_normalizing(vec![PathBuf::from("sub/new.toml"), PathBuf::from("sub/./new.toml")])]
fn apply_rejects_paths_the_record_cannot_name_once(#[case] paths: Vec<PathBuf>) {
    let s = Setup::new(&[("sub/keep.toml", OLD)]);
    let p = Plan {
        changes: paths.into_iter().map(|p| write_change(p, NEW)).collect(),
        ..Plan::default()
    };
    assert!(matches!(s.apply(&p), Err(Error::Unsafe(_))));
    assert!(!s.path("new.toml").exists());
    assert!(!s.path("sub/new.toml").exists());
    assert!(!s.backup_exists());
}

#[test]
fn a_cache_that_cannot_be_written_stops_apply_before_the_backup() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    let blocked = s.cache.path().join("not-a-directory");
    fs::write(&blocked, "").unwrap();
    assert!(apply(&s.root(), &s.plan(), &blocked).is_err());
    assert_eq!(s.read("slopvac.toml"), OLD);
    assert!(!s.backup_exists());
}

#[test]
fn restore_puts_back_the_migrations_own_file_when_its_completion_was_not_saved() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    let records = s.cache.path().join("migrate-backups");
    let set_mode = |mode| fs::set_permissions(&records, fs::Permissions::from_mode(mode)).unwrap();
    // The change goes through, but the record of its completion cannot be saved.
    let mut hook = |point: Point<'_>| {
        if matches!(point, Point::Discard(_)) {
            set_mode(0o500);
        }
    };
    assert!(s.apply_hooked(&s.plan(), &mut hook).is_err());
    set_mode(0o700);
    assert_eq!(s.read("slopvac.toml"), NEW);
    let done = s.restore(false).unwrap();
    assert_eq!((done.restored, done.kept.len()), (1, 0));
    assert_eq!(s.read("slopvac.toml"), OLD);
    assert!(!s.backup_exists());
}

#[test]
fn restore_leaves_a_target_that_holds_its_proposed_bytes_after_apply_refused_it() {
    let s = Setup::new(&[("slopvac.toml", OLD), ("a.md", DOC_OLD), ("b.md", DOC_OLD)]);
    let p = s.plan();
    let last = p.changes.last().unwrap();
    assert_eq!(last.path, Path::new("b.md"));
    let proposed = last.new.clone().unwrap();
    let inode = fs::metadata(s.path("b.md")).unwrap().ino();
    // Someone else writes exactly the proposed bytes into b.md just before apply checks it.
    let mut hook = |point: Point<'_>| {
        if point == Point::Check(Path::new("b.md")) {
            fs::write(s.path("b.md"), &proposed).unwrap();
        }
    };
    assert!(matches!(
        s.apply_hooked(&p, &mut hook),
        Err(Error::Changed(_))
    ));
    let done = s.restore(false).unwrap();
    assert_eq!(done.kept, vec![PathBuf::from("b.md")]);
    assert_eq!(s.read("b.md"), proposed);
    assert_eq!(fs::metadata(s.path("b.md")).unwrap().ino(), inode);
    assert_eq!(s.read("slopvac.toml"), OLD);
    assert_eq!(s.read("a.md"), DOC_OLD);
}

#[rstest]
#[case::refused(false)]
#[case::kept_with_force(true)]
fn restore_never_deletes_a_created_file_swapped_for_one_with_the_same_bytes(#[case] force: bool) {
    let s = Setup::new(&[(".slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    // A new file with the same bytes replaces the one apply created.
    s.write("swap.tmp", NEW);
    fs::rename(s.path("swap.tmp"), s.path("slopvac.toml")).unwrap();
    let inode = fs::metadata(s.path("slopvac.toml")).unwrap().ino();
    let result = s.restore(force);
    if force {
        assert_eq!(result.unwrap().kept, vec![PathBuf::from("slopvac.toml")]);
    } else {
        assert!(matches!(result, Err(Error::Changed(_))));
    }
    assert_eq!(s.read("slopvac.toml"), NEW);
    assert_eq!(fs::metadata(s.path("slopvac.toml")).unwrap().ino(), inode);
}

#[rstest]
#[case::hardlink(false)]
#[case::symlink(true)]
fn apply_refuses_a_staged_file_replaced_before_it_takes_the_name(#[case] symlink: bool) {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    fs::write(s.outside(), "outside\n").unwrap();
    let mut kept = None;
    let mut hook = |point: Point<'_>| {
        if point == Point::Install(Path::new("slopvac.toml")) {
            let hold = holding(&s.root());
            let staged = hold.join("new");
            fs::remove_file(&staged).unwrap();
            if symlink {
                std::os::unix::fs::symlink(s.outside(), &staged).unwrap();
            } else {
                fs::hard_link(s.outside(), &staged).unwrap();
            }
            kept = Some(hold.join("old"));
        }
    };
    assert!(matches!(
        s.apply_hooked(&s.plan(), &mut hook),
        Err(Error::Changed(_))
    ));
    assert_eq!(fs::read_to_string(s.outside()).unwrap(), "outside\n");
    // The checked original is not removed, because the name does not hold the staged file.
    assert_eq!(fs::read_to_string(kept.unwrap()).unwrap(), OLD);
}

#[test]
fn apply_keeps_an_original_replaced_before_it_is_removed() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    fs::write(s.outside(), "outside\n").unwrap();
    let mut held = None;
    let mut hook = |point: Point<'_>| {
        if point == Point::Discard(Path::new("slopvac.toml")) {
            let old = holding(&s.root()).join("old");
            link_outside(&s, &old);
            held = Some(old);
        }
    };
    let err = s.apply_hooked(&s.plan(), &mut hook).unwrap_err();
    let held = held.unwrap();
    let held = held.strip_prefix(s.root()).unwrap();
    assert!(matches!(err, Error::Changed(_)), "{err:?}");
    assert_eq!(
        err.to_string(),
        format!(
            "changed: slopvac.toml: {} changed after it was checked, so it was not removed; delete it by hand",
            held.display()
        )
    );
    assert_eq!(s.read("slopvac.toml"), NEW);
    assert_eq!(outside_links(&s), 2, "the swapped-in link was removed");
}

#[test]
fn restore_keeps_a_backup_file_replaced_before_it_is_removed() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    fs::write(s.outside(), "outside\n").unwrap();
    let copy = Path::new(BACKUP_DIR).join("0");
    let mut hook = |point: Point<'_>| {
        if point == Point::Cleanup(&copy) {
            link_outside(&s, &holding(&s.path(BACKUP_DIR)).join("0"));
        }
    };
    assert!(matches!(
        s.restore_hooked(false, &mut hook),
        Err(Error::Changed(_))
    ));
    assert_eq!(s.read("slopvac.toml"), OLD);
    assert_eq!(outside_links(&s), 2, "the swapped-in link was removed");
}

#[rstest]
#[case::staged_target("slopvac.toml")]
#[case::backup_copy(".slopvac-migrate-backup/0")]
#[case::manifest(".slopvac-migrate-backup/manifest.txt")]
fn a_failed_write_leaves_no_file_behind(#[case] failing: &str) {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    let mut hook = |point: Point<'_>| match point {
        Point::Write(p) if p == Path::new(failing) => {
            Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
        }
        _ => Ok(()),
    };
    assert!(s.apply_failing(&s.plan(), &mut hook).is_err());
    assert_eq!(s.read("slopvac.toml"), OLD);
    let names: Vec<_> = fs::read_dir(s.root())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["slopvac.toml"]);
}

fn run_cli(s: &Setup, args: &[&str]) -> (i32, String, String) {
    let root = s.root();
    let all = [root.to_str().unwrap()]
        .into_iter()
        .chain(args.iter().copied());
    let m = command().get_matches_from(std::iter::once("migrate").chain(all));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(
        &m,
        &NullLinter,
        STEERING,
        s.cache.path(),
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn the_kept_file_notice_escapes_control_characters_in_the_name() {
    let dir = "e\u{1b}[31m";
    let s = Setup::new(&[(&format!("{dir}/.slopvac.toml"), OLD)]);
    assert_eq!(run_cli(&s, &["--write"]).0, 0);
    s.write(&format!("{dir}/slopvac.toml"), EDITED);
    let (code, out, err) = run_cli(&s, &["--restore", "--force"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("kept e\\u{1b}[31m/slopvac.toml"), "{out}");
    assert!(!out.contains('\u{1b}'), "{out}");
}
