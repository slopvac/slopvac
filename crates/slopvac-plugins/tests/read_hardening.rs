//! Read paths that `doctor` and `plugins status` share must not block on a swapped-in FIFO.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![cfg(unix)]

use std::fs;
use std::sync::mpsc;
use std::time::Duration;

use slopvac_plugins::lock::Digest;
use slopvac_plugins::sync::{Store, TreeState};

const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn mkfifo(path: &std::path::Path) {
    assert!(
        std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}

/// `Store::inspect` on a store whose ref for the digest was replaced by `make_ref`.
fn inspect_with_ref(make_ref: impl FnOnce(&std::path::Path)) -> TreeState {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("refs")).unwrap();
    make_ref(&dir.path().join("refs").join(DIGEST_HEX));
    let root = dir.path().to_owned();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let digest: Digest = format!("sha256:{DIGEST_HEX}").parse().unwrap();
        let _ = tx.send(Store::new(root).inspect(&digest).unwrap());
    });
    let state = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("inspect blocked on the ref");
    drop(dir);
    state
}

#[test]
fn a_fifo_in_place_of_a_ref_reads_as_not_installed_without_blocking() {
    assert_eq!(inspect_with_ref(mkfifo), TreeState::Absent);
}

#[test]
fn a_link_from_a_ref_to_a_fifo_reads_as_not_installed_without_blocking() {
    let state = inspect_with_ref(|ref_path| {
        let fifo = ref_path.with_file_name("pipe");
        mkfifo(&fifo);
        std::os::unix::fs::symlink(&fifo, ref_path).unwrap();
    });
    assert_eq!(state, TreeState::Absent);
}

#[test]
fn an_oversize_ref_reads_as_not_installed() {
    let state = inspect_with_ref(|ref_path| fs::write(ref_path, "a".repeat(65)).unwrap());
    assert_eq!(state, TreeState::Absent);
}
