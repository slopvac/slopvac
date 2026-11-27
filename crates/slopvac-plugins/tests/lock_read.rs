//! Reading the lock of a project reports each failure as its own variant with its own code.
#![allow(
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::fs;

use slopvac_plugins::lock::{LockReadError, read_project_lock};

fn project_with_lock(text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".slopvac")).unwrap();
    fs::write(dir.path().join(".slopvac/plugins.lock"), text).unwrap();
    dir
}

#[test]
fn a_missing_lock_that_is_required_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();

    let err = read_project_lock(dir.path(), true).unwrap_err();

    assert!(matches!(err, LockReadError::Unreadable(_)), "{err:?}");
    assert_eq!(err.code(), "plugins-lock-unreadable");
}

#[test]
fn a_missing_lock_that_is_not_required_is_empty() {
    let dir = tempfile::tempdir().unwrap();

    let lock = read_project_lock(dir.path(), false).unwrap();

    assert!(lock.packs().is_empty());
}

#[test]
fn a_file_that_is_not_a_lock_is_invalid_and_its_text_names_no_path() {
    let dir = project_with_lock("this is [[[ not a lock\n");

    let err = read_project_lock(dir.path(), true).unwrap_err();

    assert!(matches!(err, LockReadError::Invalid(_)), "{err:?}");
    assert_eq!(err.code(), "plugins-lock-invalid");
    let text = err.to_string();
    assert!(!text.contains(dir.path().to_str().unwrap()), "{text}");
}
