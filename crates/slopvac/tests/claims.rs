//! `slopvac claims` through the installed binary: the report, the errors that stop a run,
//! paths kept out of messages, and no traffic through a configured proxy.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::io::ErrorKind;
use std::net::TcpListener;

mod support;

fn slopvac(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    // Tests stay off the network: the update notice may refresh its cache after lint, doctor and prime.
    cmd.env("SLOPVAC_NO_UPDATE_CHECK", "1");
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

/// The document cap, 64 MiB, written out here so a change to the shared constant is noticed.
const DOCUMENT_CAP: u64 = 67_108_864;

/// Make `name` in `dir` a sparse file of `len` bytes, which stands in for an over-cap input that
/// the run must refuse before it reads it.
fn sized_file(dir: &std::path::Path, name: &str, len: u64) {
    fs::File::create(dir.join(name))
        .expect("create")
        .set_len(len)
        .expect("set_len");
}

#[rstest::rstest]
#[case::original("old.md")]
#[case::revised("new.md")]
fn an_input_one_byte_over_the_cap_is_refused_as_a_too_large_document(#[case] big: &str) {
    assert_eq!(slopvac_fs::MAX_DOCUMENT_BYTES, DOCUMENT_CAP);
    let dir = project();
    sized_file(dir.path(), big, DOCUMENT_CAP + 1);
    let out = slopvac(dir.path())
        .args(["claims", "new.md", "--original", "old.md"])
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert_eq!(
        stderr.as_ref(),
        format!("error[document-too-large]: {big} is larger than {DOCUMENT_CAP} bytes\n"),
    );
    assert_eq!(out.stdout, Vec::<u8>::new());
}

/// Replace `dir/name` with a relative symlink to the file moved to `dir/real/name`, the way a
/// project links a shared document into place.
#[cfg(unix)]
fn relink(dir: &std::path::Path, name: &str) {
    let real = dir.join("real");
    fs::create_dir_all(&real).expect("real dir");
    fs::rename(dir.join(name), real.join(name)).expect("move");
    std::os::unix::fs::symlink(std::path::Path::new("real").join(name), dir.join(name))
        .expect("link");
}

#[cfg(unix)]
#[rstest::rstest]
#[case::original("old.md")]
#[case::revised("new.md")]
fn a_symlinked_input_is_read_like_the_file_it_points_to(#[case] linked: &str) {
    let args = ["claims", "new.md", "--original", "old.md"];
    let plain = project();
    let expected = slopvac(plain.path()).args(args).output().expect("run");
    assert_eq!(expected.status.code(), Some(0));

    let dir = project();
    relink(dir.path(), linked);
    let out = slopvac(dir.path()).args(args).output().expect("run");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, expected.stdout);
}

#[cfg(unix)]
#[rstest::rstest]
#[case::original("old.md")]
#[case::revised("new.md")]
fn a_symlink_to_a_file_over_the_cap_is_refused_under_the_name_the_user_gave(#[case] linked: &str) {
    let dir = project();
    sized_file(dir.path(), linked, DOCUMENT_CAP + 1);
    relink(dir.path(), linked);
    let out = slopvac(dir.path())
        .args(["claims", "new.md", "--original", "old.md"])
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert_eq!(
        stderr.as_ref(),
        format!("error[document-too-large]: {linked} is larger than {DOCUMENT_CAP} bytes\n"),
    );
}

#[cfg(unix)]
#[rstest::rstest]
#[case::original("old.md")]
#[case::revised("new.md")]
fn a_symlink_to_a_fifo_is_refused_without_blocking(#[case] linked: &str) {
    let dir = project();
    fs::remove_file(dir.path().join(linked)).expect("remove");
    fs::create_dir(dir.path().join("real")).expect("real dir");
    let fifo = dir.path().join("real").join(linked);
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(made.success());
    std::os::unix::fs::symlink(
        std::path::Path::new("real").join(linked),
        dir.path().join(linked),
    )
    .expect("link");
    let out = slopvac(dir.path())
        .args(["claims", "new.md", "--original", "old.md"])
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.starts_with("error[setup-io]"), "{stderr}");
}

#[test]
fn a_json_report_of_a_text_over_the_sentence_budget_is_a_refusal_notice_with_the_budget() {
    let dir = project();
    // 300000 sentences of 4 bytes ("Go. ") and a newline: 1,200,001 bytes. One sentence for
    // every 8 bytes plus 65,536 is 150,000 + 65,536 = 215,536.
    fs::write(
        dir.path().join("new.md"),
        format!("{}\n", "Go. ".repeat(300_000)),
    )
    .expect("new");
    let out = slopvac(dir.path())
        .args(["claims", "new.md", "--original", "old.md"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&out.stdout).expect("json on stdout");
    let notice = &report["notices"][0];
    assert_eq!(notice["code"], "context-budget-exceeded", "{report}");
    assert_eq!(notice["data"]["budget"], "sentences", "{report}");
    assert_eq!(notice["data"]["limit"], 215_536, "{report}");
    assert_eq!(notice["data"]["path"], "new.md", "{report}");
    let message = notice["message"].as_str().expect("message");
    assert!(!message.contains("Go."), "{message}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error[context-budget-exceeded]"),
        "{stderr}"
    );
}

#[test]
fn a_refusal_of_the_original_names_the_original_file() {
    let dir = project();
    // The same 1,200,001 bytes of 4-byte sentences, now in the original: the budget is
    // 150,000 + 65,536 = 215,536 sentences, and the path is the original's.
    fs::write(
        dir.path().join("old.md"),
        format!("{}\n", "Go. ".repeat(300_000)),
    )
    .expect("old");
    let out = slopvac(dir.path())
        .args(["claims", "new.md", "--original", "old.md"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&out.stdout).expect("json on stdout");
    let data = &report["notices"][0]["data"];
    assert_eq!(data["budget"], "sentences", "{report}");
    assert_eq!(data["limit"], 215_536, "{report}");
    assert_eq!(data["path"], "old.md", "{report}");
}
