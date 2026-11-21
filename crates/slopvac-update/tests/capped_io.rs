//! The cache and install metadata sit on user-writable paths. A FIFO, a directory or an
//! oversized file there must be refused without hanging.
#![cfg(unix)]

use std::path::Path;

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

/// Run `f` on a thread and wait for its result.
///
/// - A panic in `f` is resumed here, and the FIFO is not touched.
/// - A hang fails the test. The FIFO is opened read-write first, so a worker stuck opening it
///   can end: an `O_RDWR` open of a FIFO never blocks on Linux or macOS, which a write-only
///   open does when no reader exists. Then the worker is given a bounded time to finish.
fn run_bounded<T: Send + 'static>(
    fifo: &Path,
    hung: &str,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(value) => {
            handle.join().expect("the worker finished after sending");
            value
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Err(panic) => std::panic::resume_unwind(panic),
            Ok(()) => panic!("the worker ended without a result"),
        },
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            let _ = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(fifo);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !handle.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("{hung}");
        }
    }
}

fn within_timeout<T: Send + 'static>(fifo: &Path, f: impl FnOnce() -> T + Send + 'static) -> T {
    run_bounded(fifo, "the read hung", f)
}

#[test]
#[should_panic(expected = "worker boom")]
fn a_worker_that_panics_before_opening_the_fifo_is_reported_without_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("fifo");
    plant(Plant::Fifo, &fifo, "");
    within_timeout(&fifo, || -> () { panic!("worker boom") });
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
