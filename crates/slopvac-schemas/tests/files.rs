//! `write` and `check` against a scratch repository root.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::fs;

use slopvac_schemas::{ARTIFACTS, check, write};

fn scratch() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("schemas/v3")).unwrap();
    root
}

#[test]
fn check_passes_on_the_files_write_produced() {
    let root = scratch();
    let written = write(root.path()).unwrap();
    assert_eq!(written.len(), ARTIFACTS.len());
    assert_eq!(check(root.path()).unwrap(), []);
}

#[test]
fn check_reports_an_edited_file_with_a_diff_of_the_edit() {
    let root = scratch();
    write(root.path()).unwrap();
    let path = root.path().join("schemas/v3/common.schema.json");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("\"minimum\": 1", "\"minimum\": 0")).unwrap();
    let drift = check(root.path()).unwrap();
    assert_eq!(drift.len(), 1);
    assert_eq!(drift[0].file, "common.schema.json");
    assert!(
        drift[0].diff.contains("-          \"minimum\": 0"),
        "{}",
        drift[0].diff
    );
    assert!(
        drift[0].diff.contains("+          \"minimum\": 1"),
        "{}",
        drift[0].diff
    );
}
