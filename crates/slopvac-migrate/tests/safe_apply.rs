//! Apply and restore in a checkout whose author controls every file: links, swapped files,
//! edits, and forged, extended or replayed backups.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rstest::rstest;
use slopvac_migrate::{
    BACKUP_DIR, Change, ChangeKind, Error, NullLinter, Options, Plan, RenameTable, Restored, apply,
    apply_hooked, command, plan, restore, restore_hooked, run,
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

    fn apply_hooked(&self, p: &Plan, hook: &mut dyn FnMut(&Path)) -> Result<(), Error> {
        apply_hooked(&self.root(), p, self.cache.path(), &mut |rel| {
            hook(rel);
            Ok(())
        })
    }

    fn restore(&self, force: bool) -> Result<Restored, Error> {
        restore(&self.root(), self.cache.path(), force)
    }

    fn restore_hooked(&self, force: bool, hook: &mut dyn FnMut(&Path)) -> Result<Restored, Error> {
        restore_hooked(&self.root(), self.cache.path(), force, &mut |rel| {
            hook(rel);
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

/// Run `swap` on `name` the first time the hook reaches it.
fn once<'a>(s: &'a Setup, swap: Swap, name: &'static str) -> impl FnMut(&Path) + 'a {
    let mut done = false;
    move |rel| {
        if !done && rel == Path::new(name) {
            done = true;
            swap.run(s, name);
        }
    }
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
    let mut hook = |rel: &Path| {
        if rel == Path::new("sub/slopvac.toml") {
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
    let moved = s.outer.path().join("moved");
    fs::rename(s.root(), &moved).unwrap();
    fs::create_dir(s.root()).unwrap();
    fs::copy(moved.join("slopvac.toml"), s.path("slopvac.toml")).unwrap();
    copy_dir(&moved.join(BACKUP_DIR), &s.path(BACKUP_DIR));
    assert!(matches!(s.restore(true), Err(Error::Unsafe(_))));
    assert_eq!(s.read("slopvac.toml"), NEW);
}

#[test]
fn restore_rejects_a_replayed_older_backup() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    let old_backup = s.outer.path().join("old-backup");
    copy_dir(&s.path(BACKUP_DIR), &old_backup);
    s.restore(false).unwrap();
    s.apply(&s.plan()).unwrap();
    fs::remove_dir_all(s.path(BACKUP_DIR)).unwrap();
    copy_dir(&old_backup, &s.path(BACKUP_DIR));
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
fn restore_puts_back_a_replaced_file_whose_completion_was_not_recorded() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    let records = s.cache.path().join("migrate-backups");
    let set_mode = |mode| fs::set_permissions(&records, fs::Permissions::from_mode(mode)).unwrap();
    // The change goes through, but the record of it cannot be saved.
    let mut hook = |_: &Path| set_mode(0o500);
    assert!(s.apply_hooked(&s.plan(), &mut hook).is_err());
    set_mode(0o700);
    assert_eq!(s.read("slopvac.toml"), NEW);
    let done = s.restore(false).unwrap();
    assert_eq!((done.restored, done.kept.len()), (1, 0));
    assert_eq!(s.read("slopvac.toml"), OLD);
    assert!(!s.backup_exists());
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
