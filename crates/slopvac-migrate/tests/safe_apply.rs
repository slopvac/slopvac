//! Apply and restore in a checkout whose author controls every file: links, swapped
//! files, edits and forged backups.

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use sha2::{Digest, Sha256};
use slopvac_migrate::{BACKUP_DIR, Error, Options, Plan, RenameTable, apply, plan, restore};

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
            fs::write(s.root().join(name), text).unwrap();
        }
        s
    }

    fn root(&self) -> PathBuf {
        self.outer.path().join("proj")
    }

    fn outside(&self) -> PathBuf {
        self.outer.path().join("outside.toml")
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.root().join(name)).unwrap()
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

    fn restore(&self, force: bool) -> Result<slopvac_migrate::Restored, Error> {
        restore(&self.root(), self.cache.path(), force)
    }

    fn backup_exists(&self) -> bool {
        self.root().join(BACKUP_DIR).exists()
    }
}

#[derive(Debug, Clone, Copy)]
enum Swap {
    Hardlink,
    Symlink,
}

/// Make `target` another name for `outside`, which gets the bytes `target` holds now.
fn swap(how: Swap, target: &Path, outside: &Path) {
    fs::write(outside, fs::read(target).unwrap()).unwrap();
    fs::remove_file(target).unwrap();
    match how {
        Swap::Hardlink => fs::hard_link(outside, target).unwrap(),
        Swap::Symlink => std::os::unix::fs::symlink(outside, target).unwrap(),
    }
}

fn hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[rstest]
#[case::hardlink(Swap::Hardlink)]
#[case::symlink(Swap::Symlink)]
fn apply_refuses_a_target_that_is_another_name_for_an_outside_file(#[case] how: Swap) {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    let p = s.plan();
    swap(how, &s.root().join("slopvac.toml"), &s.outside());
    assert!(matches!(s.apply(&p), Err(Error::Unsafe(_))));
    assert_eq!(fs::read_to_string(s.outside()).unwrap(), OLD);
    assert!(!s.backup_exists());
}

#[test]
fn apply_refuses_bytes_that_changed_after_the_plan() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    let p = s.plan();
    fs::write(s.root().join("slopvac.toml"), EDITED).unwrap();
    assert!(matches!(s.apply(&p), Err(Error::Changed(_))));
    assert_eq!(s.read("slopvac.toml"), EDITED);
    assert!(!s.backup_exists());
}

#[rstest]
#[case::hardlink(Swap::Hardlink)]
#[case::symlink(Swap::Symlink)]
fn restore_refuses_a_target_that_is_another_name_for_an_outside_file(#[case] how: Swap) {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    swap(how, &s.root().join("slopvac.toml"), &s.outside());
    assert!(matches!(s.restore(true), Err(Error::Unsafe(_))));
    assert_eq!(fs::read_to_string(s.outside()).unwrap(), NEW);
}

#[test]
fn restore_refuses_a_file_edited_after_the_write_and_touches_nothing() {
    let s = Setup::new(&[("slopvac.toml", OLD), ("a.md", DOC_OLD)]);
    s.apply(&s.plan()).unwrap();
    let doc_new = s.read("a.md");
    fs::write(s.root().join("slopvac.toml"), EDITED).unwrap();
    assert!(matches!(s.restore(false), Err(Error::Changed(_))));
    assert_eq!(s.read("slopvac.toml"), EDITED);
    assert_eq!(s.read("a.md"), doc_new);
    assert!(s.backup_exists());
}

#[test]
fn restore_with_force_puts_back_a_file_edited_after_the_write() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    fs::write(s.root().join("slopvac.toml"), EDITED).unwrap();
    let done = s.restore(true).unwrap();
    assert_eq!((done.restored, done.kept.len()), (1, 0));
    assert_eq!(s.read("slopvac.toml"), OLD);
    assert!(!s.backup_exists());
}

#[test]
fn restore_keeps_a_created_file_that_was_edited() {
    let s = Setup::new(&[(".slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    fs::write(s.root().join("slopvac.toml"), EDITED).unwrap();
    let done = s.restore(true).unwrap();
    assert_eq!(done.kept, vec![PathBuf::from("slopvac.toml")]);
    assert_eq!(s.read("slopvac.toml"), EDITED);
    assert_eq!(s.read(".slopvac.toml"), OLD);
}

#[test]
fn restore_counts_a_file_already_back_to_its_original() {
    let s = Setup::new(&[("slopvac.toml", OLD), ("a.md", DOC_OLD)]);
    s.apply(&s.plan()).unwrap();
    fs::write(s.root().join("slopvac.toml"), OLD).unwrap();
    let done = s.restore(false).unwrap();
    assert_eq!(done.restored, 2);
    assert_eq!(s.read("a.md"), DOC_OLD);
}

#[derive(Debug, Clone, Copy)]
enum Forgery {
    /// The checkout ships a backup directory of its own.
    PreSeeded,
    /// A real backup gains an entry after the write.
    Appended,
}

#[rstest]
#[case::pre_seeded(Forgery::PreSeeded)]
#[case::appended(Forgery::Appended)]
fn restore_rejects_a_forged_manifest_that_names_an_unrelated_file(#[case] forgery: Forgery) {
    let s = Setup::new(&[("slopvac.toml", OLD), ("README.md", "keep\n")]);
    let backup = s.root().join(BACKUP_DIR);
    // A well-formed entry that says the migration created README.md with these bytes.
    let entry = format!("- {} README.md\n", hex(b"keep\n"));
    match forgery {
        Forgery::PreSeeded => {
            fs::create_dir(&backup).unwrap();
            fs::write(
                backup.join("manifest.txt"),
                format!("slopvac-migrate-manifest 1 {}\n{entry}", "0".repeat(32)),
            )
            .unwrap();
        }
        Forgery::Appended => {
            s.apply(&s.plan()).unwrap();
            let mut manifest = fs::read_to_string(backup.join("manifest.txt")).unwrap();
            manifest.push_str(&entry);
            fs::write(backup.join("manifest.txt"), manifest).unwrap();
        }
    }
    assert!(matches!(s.restore(true), Err(Error::Unsafe(_))));
    assert_eq!(s.read("README.md"), "keep\n");
}

#[test]
fn restore_rejects_a_backup_copy_that_does_not_match_the_manifest() {
    let s = Setup::new(&[("slopvac.toml", OLD)]);
    s.apply(&s.plan()).unwrap();
    fs::write(s.root().join(BACKUP_DIR).join("slopvac.toml"), EDITED).unwrap();
    assert!(matches!(s.restore(true), Err(Error::Unsafe(_))));
    assert_eq!(s.read("slopvac.toml"), NEW);
}
