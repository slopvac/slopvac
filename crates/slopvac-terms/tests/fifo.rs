//! A FIFO at a terms or blocklist path is refused at once. It must not block the run.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use slopvac_terms::{Blocklist, TermsError, TermsFile};

/// Run `f` on a thread and fail the test when it has not returned after five seconds.
fn within_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the read blocked on a FIFO")
}

fn mkfifo(path: &Path) {
    let made = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
}

fn refused(err: &TermsError, shown: &str) {
    match err {
        TermsError::Io { path, message } => {
            assert_eq!(path, shown);
            assert!(message.contains("not a regular file"), "{message}");
        }
        other => panic!("expected an I/O error, got {other:?}"),
    }
}

#[test]
fn a_fifo_terms_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("terms.toml");
    mkfifo(&path);
    let err = within_timeout(move || TermsFile::load_labeled(&path, "terms.toml").unwrap_err());
    refused(&err, "terms.toml");
}

#[test]
fn a_fifo_blocklist_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("block.toml");
    mkfifo(&path);
    let err = within_timeout(move || Blocklist::load_labeled(&path, "block.toml").unwrap_err());
    refused(&err, "block.toml");
}

#[test]
fn a_regular_terms_file_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("terms.toml");
    std::fs::write(&path, "schema_version = 3\n[[terms]]\nterm = \"GitHub\"\n").unwrap();
    let file = within_timeout(move || TermsFile::load_labeled(&path, "terms.toml"));
    assert!(file.is_ok(), "{file:?}");
}
