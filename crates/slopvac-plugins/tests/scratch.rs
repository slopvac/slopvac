//! The scratch directory removes read-only trees on drop.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![cfg(unix)]

mod support;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn a_sealed_tree_is_fully_removed_on_drop() {
    let scratch = support::scratch();
    let root = scratch.path().to_owned();
    let tree = root.join("tree");
    fs::create_dir_all(tree.join("rules")).unwrap();
    fs::write(tree.join("rules/a.yml"), b"x").unwrap();
    fs::write(tree.join("pack.toml"), b"x").unwrap();
    let locked = root.join("locked");
    fs::create_dir(&locked).unwrap();
    // A link to a directory outside the scratch directory must survive, unchanged.
    let outside = tempfile::tempdir().unwrap();
    fs::set_permissions(outside.path(), fs::Permissions::from_mode(0o500)).unwrap();
    symlink(outside.path(), root.join("link")).unwrap();
    for dir in [tree.join("rules"), tree.clone(), locked.clone()] {
        let mode = if dir == locked { 0o000 } else { 0o555 };
        fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).unwrap();
    }

    drop(scratch);

    assert!(!root.exists(), "{} was left behind", root.display());
    let mode = fs::metadata(outside.path()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o500, "a link target was changed");
    fs::set_permissions(outside.path(), fs::Permissions::from_mode(0o700)).unwrap();
}
