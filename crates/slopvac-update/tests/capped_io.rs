//! The cache and install metadata sit on user-writable paths. A FIFO, a directory or an
//! oversized file there must be refused without hanging.
#![cfg(unix)]

use std::path::Path;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use rstest::rstest;
use slopvac_update::install::{InstallMethod, detect};
use slopvac_update::notice::{Cache, Entry};

#[derive(Clone, Copy, Debug)]
enum Plant {
    Fifo,
    Directory,
    Oversized,
}

fn plant(kind: Plant, path: &Path, valid: &str) {
    match kind {
        Plant::Fifo => {
            let status = std::process::Command::new("mkfifo")
                .arg(path)
                .status()
                .expect("mkfifo runs");
            assert!(status.success());
        }
        Plant::Directory => std::fs::create_dir(path).unwrap(),
        Plant::Oversized => {
            // A well-formed document padded past any cap: a cap that is not enforced would parse it.
            let padded = valid.replacen('{', &format!("{{\"pad\":\"{}\",", "x".repeat(2 << 20)), 1);
            std::fs::write(path, padded).unwrap();
        }
    }
}

/// Run `f` on a thread. A hang fails the test; a FIFO is opened for writing first, so the
/// stuck thread can end.
fn within_timeout<T: Send + 'static>(fifo: &Path, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(v) => v,
        Err(RecvTimeoutError::Timeout) => {
            let _ = std::fs::OpenOptions::new().write(true).open(fifo);
            panic!("the read hung");
        }
        Err(RecvTimeoutError::Disconnected) => panic!("the read panicked"),
    }
}

#[rstest]
#[case::fifo(Plant::Fifo)]
#[case::directory(Plant::Directory)]
#[case::oversized(Plant::Oversized)]
fn a_special_or_oversized_cache_file_reads_as_missing(#[case] kind: Plant) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("update-check.json");
    plant(kind, &path, r#"{"checked_at":1,"latest":"9.9.9"}"#);
    let cache = Cache::at(path.clone());
    assert_eq!(within_timeout(&path, move || cache.read()), None);
}

#[test]
fn a_regular_cache_file_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::at(dir.path().join("update-check.json"));
    let entry = Entry {
        checked_at: 7,
        latest: "9.9.9".into(),
    };
    cache.write(&entry).unwrap();
    assert_eq!(cache.read(), Some(entry));
}

fn receipt(prefix: &Path) -> String {
    serde_json::json!({
        "install_prefix": prefix,
        "source": {"app_name": "slopvac"},
    })
    .to_string()
}

#[rstest]
#[case::fifo(Plant::Fifo)]
#[case::directory(Plant::Directory)]
#[case::oversized(Plant::Oversized)]
fn a_special_or_oversized_receipt_does_not_make_an_installer_install(#[case] kind: Plant) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    std::fs::create_dir(&cfg).unwrap();
    let prefix = tmp.path().join("prefix");
    let exe = prefix.join("bin/slopvac");
    let file = cfg.join("slopvac-receipt.json");
    plant(kind, &file, &receipt(&prefix));
    let cfg_var = cfg.display().to_string();
    let method = within_timeout(&file, move || {
        detect(&exe, &|k| {
            (k == "AXOUPDATER_CONFIG_PATH").then(|| cfg_var.clone())
        })
    });
    assert_eq!(method, InstallMethod::Unknown);
}

#[test]
fn a_regular_receipt_makes_an_installer_install() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    std::fs::create_dir(&cfg).unwrap();
    let prefix = tmp.path().join("prefix");
    std::fs::write(cfg.join("slopvac-receipt.json"), receipt(&prefix)).unwrap();
    let cfg_var = cfg.display().to_string();
    let method = detect(&prefix.join("bin/slopvac"), &|k| {
        (k == "AXOUPDATER_CONFIG_PATH").then(|| cfg_var.clone())
    });
    assert_eq!(method, InstallMethod::Installer);
}
