//! Scratch directories for tests that install read-only trees.
//!
//! The extractor seals every tree it installs (directories `0555`, files `0444`), and a
//! test may also set a directory to `0000`. `tempfile::TempDir` ignores the error from
//! removing such a tree on drop, so the directory stays in the temp directory for good.
//! [`Scratch`] restores owner access first, so a test that passes leaves nothing behind.
#![allow(
    dead_code,
    unreachable_pub,
    missing_docs,
    reason = "shared across test binaries"
)]

use std::fs;
use std::ops::Deref;
use std::path::Path;

use tempfile::TempDir;

/// A [`TempDir`] that deletes the tree under it even when the tree is read-only.
pub struct Scratch(TempDir);

/// Create a scratch directory under the temp directory.
pub fn scratch() -> Scratch {
    Scratch(tempfile::tempdir().unwrap())
}

impl Deref for Scratch {
    type Target = TempDir;
    fn deref(&self) -> &TempDir {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // The field drops after this body, and `TempDir` removes the tree then.
        make_removable(self.0.path());
    }
}

/// Give the owner access to every real directory under `path`. Links are never followed or
/// changed, and failures are ignored: the removal that follows reports nothing either way.
#[cfg(unix)]
fn make_removable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if !meta.is_dir() {
        return;
    }
    let mode = meta.permissions().mode();
    if mode & 0o700 != 0o700 {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o700));
    }
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            make_removable(&entry.path());
        }
    }
}

#[cfg(not(unix))]
fn make_removable(_: &Path) {}
