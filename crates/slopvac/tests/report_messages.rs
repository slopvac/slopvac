//! Every `report` error takes its code and text from the message catalog.
//!
//! Each test runs the real binary into one failure and checks the `error[<code>]: <text>` line
//! against the entry registered for that code: the code must be registered, and the text must be
//! the registered template with its parameters filled in.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use slopvac_cli::report::messages::ReportError;
use slopvac_messages::{Catalog, Entry};

fn entry(code: &str) -> &'static Entry {
    ReportError::entries()
        .iter()
        .find(|e| e.code == code)
        .unwrap_or_else(|| panic!("{code} is not registered"))
}

/// The literal text of a template before its first placeholder.
fn literal_prefix(template: &str) -> &str {
    template.split('{').next().unwrap_or_default()
}

/// The `(code, message)` of the first `error[...]` line on stderr.
fn error_line(stderr: &[u8]) -> (String, String) {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .find(|l| l.starts_with("error["))
        .unwrap_or_else(|| panic!("no error line in {text:?}"));
    let rest = line.strip_prefix("error[").unwrap();
    let (code, message) = rest.split_once("]: ").unwrap();
    (code.to_owned(), message.to_owned())
}

fn assert_from_catalog(code: &str, args: &[&str], path: &std::path::Path) {
    let output = Command::cargo_bin("slopvac")
        .unwrap()
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .env_remove("SLOPVAC_OFFLINE")
        .arg("report")
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{code}");
    let (got, message) = error_line(&output.stderr);
    assert_eq!(got, code);
    let template = entry(code).template;
    let prefix = literal_prefix(template);
    assert!(
        message.starts_with(prefix),
        "{code}: {message:?} does not start with {prefix:?}"
    );
    if !template.contains('{') {
        assert_eq!(message, template, "{code}");
    }
}

fn bug(extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "bug",
        "--title",
        "crash on empty input",
        "--field",
        "agent=Claude Code",
        "--field",
        "model=test-model",
        "--field",
        "harness=test-harness 1.0",
        "--field",
        "latest=Yes",
        "--field",
        "duplicates=true",
        "--field",
        "environment={\"ok\":true}",
        "--field",
        "command=slopvac lint --stdin",
        "--field",
        "what-happened=it crashed",
        "--field",
        "expected=no crash",
        "--field",
        "reporter=true",
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    args
}

fn fake_gh(dir: &std::path::Path, script: &str) {
    let gh = dir.join("gh");
    std::fs::write(&gh, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn run_bug(code: &str, extra: &[&str], path: &std::path::Path) {
    let args = bug(extra);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    assert_from_catalog(code, &refs, path);
}

#[test]
fn missing_fields_come_from_the_catalog() {
    let empty = tempfile::tempdir().unwrap();
    assert_from_catalog("report-incomplete", &["bug", "--title", "x"], empty.path());
}

#[test]
fn an_invalid_argument_comes_from_the_catalog() {
    let empty = tempfile::tempdir().unwrap();
    assert_from_catalog(
        "report-invalid",
        &["bug", "--title", "x", "--field", "no-equals-sign"],
        empty.path(),
    );
}

#[test]
fn an_unreadable_sample_comes_from_the_catalog() {
    let empty = tempfile::tempdir().unwrap();
    let missing = empty.path().join("absent.txt");
    run_bug(
        "report-sample-unreadable",
        &["--include-text", missing.to_str().unwrap()],
        empty.path(),
    );
}

#[test]
fn a_refused_body_file_comes_from_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let body = dir.path().join("no-such-directory").join("body.md");
    run_bug(
        "report-body-file",
        &["--body-file", body.to_str().unwrap()],
        dir.path(),
    );
}

#[test]
fn a_missing_gh_comes_from_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let body = dir.path().join("body.md");
    run_bug(
        "report-gh-missing",
        &["--body-file", body.to_str().unwrap(), "--gh", "--yes"],
        dir.path(),
    );
}

#[test]
fn a_gh_without_login_comes_from_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    fake_gh(dir.path(), "exit 1");
    let body = dir.path().join("body.md");
    run_bug(
        "report-gh-not-logged-in",
        &["--body-file", body.to_str().unwrap(), "--gh", "--yes"],
        dir.path(),
    );
}

#[test]
fn a_failing_gh_create_comes_from_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    fake_gh(dir.path(), "[ \"$1\" = auth ] && exit 0\nexit 3");
    let body = dir.path().join("body.md");
    run_bug(
        "report-gh-failed",
        &["--body-file", body.to_str().unwrap(), "--gh", "--yes"],
        dir.path(),
    );
}

#[test]
fn an_offline_gh_run_comes_from_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let body = dir.path().join("body.md");
    let args = bug(&["--body-file", body.to_str().unwrap(), "--gh", "--yes"]);
    let output = Command::cargo_bin("slopvac")
        .unwrap()
        .env("PATH", dir.path())
        .env("NO_COLOR", "1")
        .env("SLOPVAC_OFFLINE", "1")
        .arg("report")
        .args(&args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let (code, message) = error_line(&output.stderr);
    assert_eq!(code, "offline-network-needed");
    // The code lives in the `slopvac-update` catalog, whose template takes the command name.
    assert!(
        slopvac_update::error_entries()
            .iter()
            .any(|e| e.code == "offline-network-needed"),
        "offline-network-needed is not registered"
    );
    assert_eq!(message, "offline: report --gh --yes needs the network");
}
