//! Atomic, symlink-safe saves of a terms file.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::path::{Path, PathBuf};

use slopvac_terms::{TermEntry, TermsDoc};

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("slopvac-terms-save-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn entry(term: &str) -> TermEntry {
    TermEntry {
        term: term.to_owned(),
        variants: vec![],
        kind: None,
        ignore_case: false,
        case_sensitive: false,
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

const ORIGINAL: &str = "# keep\nschema_version = 3\n\n[[terms]]\nterm = \"Acme\"\n";

fn doc_with_widget(file: &Path) -> TermsDoc {
    let mut doc = TermsDoc::open(file).unwrap();
    doc.add(&entry("Widget")).unwrap();
    doc
}

#[test]
fn a_write_that_fails_part_way_leaves_the_original_and_no_temp_file() {
    let dir = scratch("partial");
    let file = dir.join("terms.toml");
    std::fs::write(&file, ORIGINAL).unwrap();
    let doc = doc_with_widget(&file);

    let result = doc.save_with(&file, |f, bytes| {
        f.write_all(&bytes[..8])?;
        Err(std::io::Error::other("disk full"))
    });

    let err = result.expect_err("the save must fail");
    assert!(err.to_string().contains("disk full"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), ORIGINAL);
    assert_eq!(names(&dir), ["terms.toml"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_failed_replace_keeps_the_target_and_removes_the_temp_file() {
    let dir = scratch("replace");
    let target = dir.join("terms.toml");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("keep"), "x").unwrap();
    let mut doc = TermsDoc::default();
    doc.add(&entry("Acme")).unwrap();

    // Renaming a file over a directory fails after the temp file was written.
    let err = doc
        .save(&target)
        .expect_err("replacing a directory must fail");
    assert!(matches!(err, slopvac_terms::TermsError::Io { .. }), "{err}");
    assert_eq!(names(&dir), ["terms.toml"]);
    assert_eq!(names(&target), ["keep"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_planted_symlink_at_the_old_temp_name_is_never_written_through() {
    use std::os::unix::fs::symlink;

    let dir = scratch("symlink-temp");
    let file = dir.join("terms.toml");
    std::fs::write(&file, ORIGINAL).unwrap();
    let victim = dir.join("victim.txt");
    std::fs::write(&victim, "precious").unwrap();
    // The names the previous implementation used: PID-based and predictable.
    let pid = std::process::id();
    for planted in [
        format!(".terms.toml.tmp-{pid}"),
        format!(".terms.tmp-{pid}"),
    ] {
        symlink(&victim, dir.join(planted)).unwrap();
    }
    let doc = doc_with_widget(&file);

    doc.save(&file).unwrap();

    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
    assert!(std::fs::read_to_string(&file).unwrap().contains("Widget"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn existing_files_with_other_names_are_left_alone() {
    let dir = scratch("collision");
    let file = dir.join("terms.toml");
    std::fs::write(&file, ORIGINAL).unwrap();
    std::fs::write(dir.join(".terms-abc.tmp"), "someone else's").unwrap();
    doc_with_widget(&file).save(&file).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join(".terms-abc.tmp")).unwrap(),
        "someone else's"
    );
    assert_eq!(names(&dir), [".terms-abc.tmp", "terms.toml"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_symlinked_terms_file_keeps_its_link_and_updates_the_referent() {
    use std::os::unix::fs::symlink;

    let dir = scratch("symlinked");
    let real = dir.join("real.toml");
    std::fs::write(&real, ORIGINAL).unwrap();
    let link = dir.join("terms.toml");
    symlink(&real, &link).unwrap();

    doc_with_widget(&link).save(&link).unwrap();

    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(std::fs::read_to_string(&real).unwrap().contains("Widget"));
    assert_eq!(names(&dir), ["real.toml", "terms.toml"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_save_keeps_the_file_mode() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("mode");
    let file = dir.join("terms.toml");
    std::fs::write(&file, ORIGINAL).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    doc_with_widget(&file).save(&file).unwrap();
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_empty_new_document_saves_an_explicit_terms_array_that_reloads() {
    let dir = scratch("empty-new");
    let file = dir.join("terms.toml");
    TermsDoc::default().save(&file).unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "schema_version = 3\nterms = []\n"
    );
    assert_eq!(slopvac_terms::TermsFile::load(&file).unwrap().terms, []);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn removing_the_last_table_term_saves_a_file_that_reloads() {
    let dir = scratch("remove-last");
    let file = dir.join("terms.toml");
    std::fs::write(&file, ORIGINAL).unwrap();
    let mut doc = TermsDoc::open(&file).unwrap();
    assert!(doc.remove("Acme"));
    doc.save(&file).unwrap();
    assert_eq!(slopvac_terms::TermsFile::load(&file).unwrap().terms, []);
    // The live document keeps the table form for the next add.
    doc.add(&entry("Widget")).unwrap();
    assert!(doc.render().contains("[[terms]]"), "{}", doc.render());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn add_rejects_entries_the_parser_would_reject_and_changes_nothing() {
    let dir = scratch("invalid-add");
    let file = dir.join("terms.toml");
    std::fs::write(&file, ORIGINAL).unwrap();
    let bad_kind = TermEntry {
        kind: Some("Proper Noun".to_owned()),
        ..entry("A")
    };
    let empty_variant = TermEntry {
        variants: vec![String::new()],
        ..entry("B")
    };
    let repeated_variant = TermEntry {
        variants: vec!["x".to_owned(), "x".to_owned()],
        ..entry("C")
    };
    let both_modes = TermEntry {
        ignore_case: true,
        case_sensitive: true,
        ..entry("D")
    };
    for bad in [bad_kind, empty_variant, repeated_variant, both_modes] {
        let mut doc = TermsDoc::open(&file).unwrap();
        assert!(doc.add(&bad).is_err(), "{bad:?}");
        assert_eq!(doc.render(), ORIGINAL, "{bad:?}");
    }
    assert_eq!(std::fs::read_to_string(&file).unwrap(), ORIGINAL);
    std::fs::remove_dir_all(&dir).unwrap();
}
