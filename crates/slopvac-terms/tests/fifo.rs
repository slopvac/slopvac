//! A FIFO at a terms or blocklist path is refused at once. It must not block the run.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use slopvac_terms::{Blocklist, TermsDoc, TermsError, TermsFile};

/// Run `f` on a worker thread. When it has not returned after five seconds, open the FIFO's
/// write end so a blocked reader wakes, join the worker, then fail.
fn within_timeout<T: Send + 'static>(fifo: &Path, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    let got = rx.recv_timeout(Duration::from_secs(5));
    if got.is_err() {
        drop(std::fs::OpenOptions::new().write(true).open(fifo));
    }
    worker.join().unwrap();
    got.expect("the read blocked on a FIFO")
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
    let fifo = path.clone();
    let err = within_timeout(&fifo, move || {
        TermsFile::load_labeled(&path, "terms.toml").unwrap_err()
    });
    refused(&err, "terms.toml");
}

#[test]
fn a_fifo_blocklist_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("block.toml");
    mkfifo(&path);
    let fifo = path.clone();
    let err = within_timeout(&fifo, move || {
        Blocklist::load_labeled(&path, "block.toml").unwrap_err()
    });
    refused(&err, "block.toml");
}

#[test]
fn a_regular_terms_file_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("terms.toml");
    std::fs::write(&path, "schema_version = 3\n[[terms]]\nterm = \"GitHub\"\n").unwrap();
    let probe = path.clone();
    let file = within_timeout(&probe, move || TermsFile::load_labeled(&path, "terms.toml"));
    assert!(file.is_ok(), "{file:?}");
}

#[test]
fn a_fifo_terms_document_is_refused_when_opened_for_editing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("terms.toml");
    mkfifo(&path);
    let fifo = path.clone();
    let err = within_timeout(&fifo, move || {
        TermsDoc::open_labeled(&path, "terms.toml").err().unwrap()
    });
    refused(&err, "terms.toml");
}

/// A link to nothing is a missing file: opening it for editing starts an empty document.
#[test]
fn a_dangling_symlink_terms_file_is_missing_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("terms.toml");
    std::os::unix::fs::symlink(dir.path().join("gone.toml"), &path).unwrap();
    let probe = path.clone();
    let doc = within_timeout(&probe, move || TermsDoc::open_labeled(&path, "terms.toml"));
    assert!(doc.is_ok(), "{:?}", doc.err());
}

/// A link to a regular file reads the file behind it.
#[test]
fn a_symlinked_terms_file_loads_through_the_link() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.toml");
    std::fs::write(&real, "schema_version = 3\n[[terms]]\nterm = \"GitHub\"\n").unwrap();
    let link = dir.path().join("terms.toml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let probe = link.clone();
    let file = within_timeout(&probe, move || TermsFile::load_labeled(&link, "terms.toml"));
    assert!(file.is_ok(), "{file:?}");
}
