//! A FIFO at a style path is refused at once. It must not block the run.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

mod common;

use slopvac_styles::{DirSource, StyleError, StyleSource};

#[test]
fn a_fifo_style_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(dir.path().join("loud.toml"))
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
    let source = DirSource::new(dir.path());
    let fifo = dir.path().join("loud.toml");
    let err = common::within_timeout(&fifo, move || source.load("loud").unwrap_err());
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
    let probe = dir.path().join("calm.toml");
    let style = common::within_timeout(&probe, move || source.load("calm"));
    assert!(style.is_ok(), "{style:?}");
}

/// A link to nothing is an unknown style, as before.
#[test]
fn a_dangling_symlink_style_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("calm.toml");
    std::os::unix::fs::symlink(dir.path().join("gone.toml"), &link).unwrap();
    let source = DirSource::new(dir.path());
    let err = common::within_timeout(&link, move || source.load("calm").unwrap_err());
    assert!(
        matches!(err, StyleError::NotFound(ref n) if n == "calm"),
        "{err:?}"
    );
}

/// A link to a regular file loads the style behind it.
#[test]
fn a_symlinked_style_loads_through_the_link() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.txt");
    std::fs::write(&real, "schema_version = 3\ndescription = \"Calm\"\n").unwrap();
    let link = dir.path().join("calm.toml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let source = DirSource::new(dir.path());
    let style = common::within_timeout(&link, move || source.load("calm"));
    assert!(style.is_ok(), "{style:?}");
}
