//! `slopvac claims` through the installed binary: the report, the errors that stop a run,
//! paths kept out of messages, and no network use.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::net::TcpListener;

mod support;

fn slopvac(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    cmd.current_dir(dir);
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1");
    cmd
}

/// A project directory whose revision changes one number and drops one sentence.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir_in(support::neutral_dir()).expect("tempdir");
    fs::write(
        dir.path().join("old.md"),
        "The cache holds 10 entries. It evicts the oldest entry first.\n",
    )
    .expect("old");
    fs::write(dir.path().join("new.md"), "The cache holds 20 entries.\n").expect("new");
    dir
}

#[test]
fn a_text_report_lists_the_changed_claim_and_its_number() {
    let dir = project();
    slopvac(dir.path())
        .args([
            "claims",
            "new.md",
            "--original",
            "old.md",
            "--format",
            "text",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("The cache holds 20 entries."))
        .stdout(predicate::str::contains("removed"))
        .stdout(predicate::str::contains("added"));
}

#[test]
fn the_default_report_is_json_with_claims_and_dropped_facts() {
    let dir = project();
    let out = slopvac(dir.path())
        .args(["claims", "new.md", "--original", "old.md"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(report["claims"].is_array(), "{report}");
    assert!(report["dropped"].is_array(), "{report}");
}

#[test]
fn an_unreadable_original_is_reported_without_its_directory() {
    let dir = project();
    let out = slopvac(dir.path())
        .args([
            "claims",
            "new.md",
            "--original",
            "/no-such-directory-for-claims/x.md",
        ])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[setup-io]"), "{stderr}");
    assert!(stderr.contains("<external>/x.md"), "{stderr}");
    assert!(!stderr.contains("no-such-directory-for-claims"), "{stderr}");
}

/// The nearest thing to a network sandbox the suite has: every proxy variable points at a
/// local listener, so any outbound HTTP attempt shows up as a connection to it.
#[test]
fn a_run_with_the_update_check_off_makes_no_connection() {
    let dir = project();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let proxy = format!("http://{}", listener.local_addr().expect("addr"));
    let mut cmd = slopvac(dir.path());
    for var in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        cmd.env(var, &proxy);
    }
    cmd.env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .args(["claims", "new.md", "--original", "old.md"])
        .assert()
        .code(0);
    assert!(
        listener.accept().is_err(),
        "claims opened a network connection"
    );
}
