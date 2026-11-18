//! A FIFO at a style path is refused at once. It must not block the run.

use std::sync::mpsc;
use std::time::Duration;

use slopvac_styles::{DirSource, StyleError, StyleSource};

/// Run `f` on a thread and fail the test when it has not returned after five seconds.
fn within_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the read blocked on a FIFO")
}

#[test]
fn a_fifo_style_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(dir.path().join("loud.toml"))
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
    let source = DirSource::new(dir.path());
    let err = within_timeout(move || source.load("loud").unwrap_err());
    match err {
        StyleError::Io { style, message } => {
            assert_eq!(style, "loud");
            assert!(message.contains("not a regular file"), "{message}");
        }
        other => panic!("expected an I/O error, got {other:?}"),
    }
}

#[test]
fn a_regular_style_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("calm.toml"),
        "schema_version = 3\ndescription = \"Calm\"\n",
    )
    .unwrap();
    let source = DirSource::new(dir.path());
    let style = within_timeout(move || source.load("calm"));
    assert!(style.is_ok(), "{style:?}");
}
