//! Installed-binary tests for the document size cap (policies.md 1.9): an input one byte over
//! 64 MiB is refused whole with `document-too-large` and exit 2. The reader's boundary
//! (exactly at the cap) is tested in `slopvac-fs`.
//!
//! `lint` accepts `--fix` and `--diff-base` but does not yet pass them to the engine, so those
//! refusals have no CLI test.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

mod support;

use assert_cmd::Command;
use predicates::prelude::*;

const CAP: usize = 64 * 1024 * 1024;

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    cmd.current_dir(support::neutral_dir()).env("NO_COLOR", "1");
    for var in ["SLOPVAC_OFFLINE", "SLOPVAC_LOG", "FORCE_COLOR"] {
        cmd.env_remove(var);
    }
    cmd
}

#[test]
fn stdin_over_the_cap_is_refused() {
    slopvac()
        .args(["lint", "--stdin"])
        .write_stdin(vec![b'a'; CAP + 1])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("document-too-large"))
        .stderr(predicate::str::contains("<stdin>"));
}

#[test]
fn file_over_the_cap_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("big.md");
    std::fs::File::create(&file)
        .and_then(|f| f.set_len(CAP as u64 + 1))
        .expect("sparse file");
    slopvac()
        .current_dir(dir.path())
        .args(["lint", "big.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("document-too-large"))
        .stderr(predicate::str::contains("big.md"));
}
