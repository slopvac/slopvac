//! Reading config files: bounded, regular files only, and never blocking.

use std::fs;
use std::path::Path;
use std::process::Command;

use slopvac_config::{Code, ConfigError, Discovery, NoPresets, load};

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    // A `.git` entry ends the upward walk inside the temp directory.
    fs::create_dir(dir.path().join(".git")).unwrap();
    dir
}

fn io_message(err: &ConfigError) -> String {
    match err {
        ConfigError::Io { .. } => err.to_string(),
        other => panic!("expected an I/O error, got {other}"),
    }
}

#[test]
fn a_directory_named_slopvac_toml_is_an_error_not_a_missing_config() {
    let dir = project();
    fs::create_dir(dir.path().join("slopvac.toml")).unwrap();
    let err = Discovery::new(&NoPresets).nearest(dir.path()).unwrap_err();
    assert_eq!(err.code(), Code::Io);
    assert_eq!(
        io_message(&err),
        "cannot read slopvac.toml: not a regular file"
    );
}

#[test]
fn an_oversize_config_is_refused_without_reading_it_all() {
    let dir = project();
    fs::write(dir.path().join("slopvac.toml"), vec![b'#'; 1024 * 1024 + 1]).unwrap();
    let err = Discovery::new(&NoPresets).nearest(dir.path()).unwrap_err();
    assert_eq!(
        io_message(&err),
        "cannot read slopvac.toml: larger than 1048576 bytes"
    );
    // Exactly at the limit is read (and then judged on its content).
    let mut body = b"schema_version = 3\n#".to_vec();
    body.resize(1024 * 1024, b'a');
    fs::write(dir.path().join("slopvac.toml"), body).unwrap();
    assert!(Discovery::new(&NoPresets).nearest(dir.path()).is_ok());
}

#[cfg(unix)]
#[test]
fn a_fifo_config_or_extended_file_does_not_hang_the_loader() {
    let dir = project();
    let fifo = |name: &str| {
        let path = dir.path().join(name);
        assert!(
            Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        path
    };
    fifo("slopvac.toml");
    let (tx, rx) = std::sync::mpsc::channel();
    let start = dir.path().to_owned();
    std::thread::spawn(move || {
        let err = Discovery::new(&NoPresets).nearest(&start).unwrap_err();
        let _ = tx.send(err.to_string());
    });
    let text = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the loader blocked on a FIFO");
    assert_eq!(text, "cannot read slopvac.toml: not a regular file");

    // An extended file that is a FIFO fails the same way, as a failed `extends` entry.
    fs::remove_file(dir.path().join("slopvac.toml")).unwrap();
    fifo("base.toml");
    fs::write(
        dir.path().join("slopvac.toml"),
        "schema_version = 3\nextends = [\"base.toml\"]\n",
    )
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let path = dir.path().join("slopvac.toml");
    std::thread::spawn(move || {
        let err = load(&path, &NoPresets).unwrap_err();
        let _ = tx.send(err.code());
    });
    let code = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the loader blocked on an extended FIFO");
    assert_eq!(code, Code::ExtendsMissing);
}

#[test]
fn a_symlink_to_a_regular_config_still_loads() {
    #[cfg(unix)]
    {
        let dir = project();
        fs::write(dir.path().join("real.toml"), "schema_version = 3\n").unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("real.toml"),
            dir.path().join("slopvac.toml"),
        )
        .unwrap();
        assert!(
            Discovery::new(&NoPresets)
                .nearest(dir.path())
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn a_missing_config_is_not_an_error() {
    let dir = project();
    assert!(
        Discovery::new(&NoPresets)
            .nearest(dir.path())
            .unwrap()
            .is_none()
    );
}

#[test]
fn invalid_utf8_names_the_cause() {
    let dir = project();
    let path: &Path = &dir.path().join("slopvac.toml");
    fs::write(path, [0xff, 0xfe, 0x00]).unwrap();
    let err = Discovery::new(&NoPresets).nearest(dir.path()).unwrap_err();
    assert_eq!(
        io_message(&err),
        "cannot read slopvac.toml: not valid UTF-8"
    );
}
