//! `slopvac claims` through the installed binary: the report, the errors that stop a run,
//! paths kept out of messages, and no traffic through a configured proxy.

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::io::ErrorKind;
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
    let out = slopvac(dir.path())
        .args([
            "claims",
            "new.md",
            "--original",
            "old.md",
            "--format",
            "text",
        ])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    // The revised sentence is a changed claim: 10 left, 20 arrived. The second original
    // sentence has no counterpart and is reported as a dropped fact.
    let expected = "changed line 1: The cache holds 20 entries.\n  removed number 10\n  added number 20\ndropped fact line 1: It evicts the oldest entry first.\n";
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}

#[test]
fn the_default_report_is_json_with_the_changed_claim_and_the_dropped_fact() {
    let dir = project();
    let out = slopvac(dir.path())
        .args(["claims", "new.md", "--original", "old.md"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&out.stdout).expect("json");
    let claims = report["claims"].as_array().expect("claims array");
    assert_eq!(claims.len(), 1, "{report}");
    assert_eq!(claims[0]["text"], "The cache holds 20 entries.");
    assert_eq!(claims[0]["change"], "changed");
    assert_eq!(claims[0]["original"], "The cache holds 10 entries.");
    assert_eq!(
        claims[0]["literals_removed"],
        serde_json::json!([{"kind": "number", "text": "10"}])
    );
    assert_eq!(
        claims[0]["literals_added"],
        serde_json::json!([{"kind": "number", "text": "20"}])
    );
    let dropped = report["dropped"].as_array().expect("dropped array");
    assert_eq!(dropped.len(), 1, "{report}");
    assert_eq!(dropped[0]["text"], "It evicts the oldest entry first.");
    assert_eq!(dropped[0]["fact_candidate"], true);
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

/// A proxy check, not a network sandbox: every proxy variable points at a local listener,
/// so only proxied HTTP traffic would show up. A direct connection would not.
#[test]
fn a_run_with_the_update_check_off_sends_nothing_through_a_proxy() {
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
    let accepted = listener.accept();
    assert!(
        matches!(&accepted, Err(e) if e.kind() == ErrorKind::WouldBlock),
        "claims sent traffic through the proxy: {accepted:?}"
    );
}
