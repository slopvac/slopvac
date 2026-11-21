//! Installed-binary tests for `lint --open` (compat rows `opt:lint:--open` and the open half of
//! `behavior:report-out-open`).
//!
//! The opener is the platform program found on `PATH` (`open` on macOS, `xdg-open` on Linux).
//! Each test puts a fake one first on `PATH`; it records its arguments to a file.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![cfg(unix)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const TEXT: &str = "one two three\n";

fn opener_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

/// A directory holding a fake opener that appends its arguments to `calls`, and exits `status`.
struct FakeOpener {
    dir: tempfile::TempDir,
}

impl FakeOpener {
    fn new(status: i32) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join(opener_name());
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> \"{}/calls\"; done\nexit {status}\n",
                dir.path().display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.dir.path().join("calls"))
            .map(|t| t.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

fn slopvac(opener: &FakeOpener) -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    // Tests stay off the network: the update notice may refresh its cache after lint, doctor and prime.
    cmd.env("SLOPVAC_NO_UPDATE_CHECK", "1");
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
        "CI",
    ] {
        cmd.env_remove(var);
    }
    let path = format!(
        "{}:{}",
        opener.dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    cmd.env("NO_COLOR", "1").env("PATH", path);
    cmd
}

/// Lint a one-line document. `--open` is not allowed with `--stdin` (stdin.rs), so a file; it
/// sits in the fake opener's directory, which the test removes when it ends.
fn lint_doc(opener: &FakeOpener, args: &[&str]) -> Command {
    let doc = opener.dir.path().join("doc.md");
    fs::write(&doc, TEXT).unwrap();
    let mut cmd = slopvac(opener);
    cmd.arg("lint").arg(&doc).args(args);
    cmd
}

#[test]
fn open_with_out_opens_the_written_file() {
    let opener = FakeOpener::new(0);
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("report.html");
    lint_doc(&opener, &["--open", "--out"])
        .arg(&target)
        .assert()
        .code(0)
        .stdout(format!("report: {}\n", target.display()));
    assert!(target.is_file());
    assert_eq!(
        opener.calls(),
        [target.canonicalize().unwrap().display().to_string()]
    );
}

#[test]
fn open_without_out_writes_an_html_page_to_a_temporary_file_and_opens_it() {
    let opener = FakeOpener::new(0);
    let out = lint_doc(&opener, &["--open"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    let path = PathBuf::from(text.strip_prefix("report: ").unwrap().trim_end());
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".html"),
        "{path:?}"
    );
    let page = fs::read_to_string(&path).unwrap();
    assert!(page.to_ascii_lowercase().starts_with("<!doctype html>"));
    assert_eq!(
        opener.calls(),
        [path.canonicalize().unwrap().display().to_string()]
    );
    fs::remove_file(&path).unwrap();
}

#[test]
fn out_alone_never_opens_anything() {
    let opener = FakeOpener::new(0);
    let dir = tempfile::tempdir().unwrap();
    lint_doc(&opener, &["--out"])
        .arg(dir.path().join("r.html"))
        .assert()
        .code(0);
    assert_eq!(opener.calls(), Vec::<String>::new());
}

#[test]
fn ci_writes_the_report_but_does_not_open_it() {
    let opener = FakeOpener::new(0);
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("report.html");
    lint_doc(&opener, &["--open", "--out"])
        .arg(&target)
        .env("CI", "true")
        .assert()
        .code(0)
        .stderr("not opening the report: CI is set.\n");
    assert!(target.is_file());
    assert_eq!(opener.calls(), Vec::<String>::new());
}

#[test]
fn a_failing_opener_is_a_warning_not_a_failed_run() {
    let opener = FakeOpener::new(3);
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("report.html");
    lint_doc(&opener, &["--open", "--out"])
        .arg(&target)
        .assert()
        .code(0)
        .stderr(predicate::str::starts_with(
            "warning: could not open the report: ",
        ));
    assert!(target.is_file());
}

#[test]
fn a_missing_opener_is_a_warning_not_a_failed_run() {
    let opener = FakeOpener::new(0);
    fs::remove_file(opener.dir.path().join(opener_name())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = lint_doc(&opener, &["--open", "--out"]);
    cmd.arg(dir.path().join("r.html"))
        // Only the empty fake directory: no opener anywhere.
        .env("PATH", opener.dir.path());
    cmd.assert().code(0).stderr(predicate::str::starts_with(
        "warning: could not open the report: ",
    ));
}

#[test]
fn open_needs_an_html_report() {
    let opener = FakeOpener::new(0);
    lint_doc(&opener, &["--open", "--format", "json"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[usage]: --open shows an HTML report",
        ));
    assert_eq!(opener.calls(), Vec::<String>::new());
}

#[test]
fn open_with_an_explicit_html_format_is_accepted() {
    let opener = FakeOpener::new(0);
    let dir = tempfile::tempdir().unwrap();
    let target: &Path = &dir.path().join("r.html");
    lint_doc(&opener, &["--open", "--format", "html", "--out"])
        .arg(target)
        .assert()
        .code(0);
    assert_eq!(opener.calls().len(), 1);
}

#[test]
fn a_report_path_with_spaces_reaches_the_opener_as_one_argument() {
    let opener = FakeOpener::new(0);
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("my report; echo hi.html");
    lint_doc(&opener, &["--open", "--out"])
        .arg(&target)
        .assert()
        .code(0);
    assert_eq!(
        opener.calls(),
        [target.canonicalize().unwrap().display().to_string()]
    );
}

#[test]
fn a_report_path_that_starts_with_a_dash_reaches_the_opener_as_an_absolute_path() {
    let opener = FakeOpener::new(0);
    let work = tempfile::tempdir().unwrap();
    let doc = work.path().join("doc.md");
    fs::write(&doc, TEXT).unwrap();
    let mut cmd = slopvac(&opener);
    cmd.current_dir(work.path())
        .arg("lint")
        .arg(&doc)
        .args(["--open", "--out=-f"])
        .assert()
        .code(0);
    assert!(work.path().join("-f").is_file());
    let calls = opener.calls();
    assert_eq!(calls.len(), 1);
    let arg = Path::new(&calls[0]);
    assert!(arg.is_absolute(), "the opener got {arg:?}");
    assert_eq!(arg.file_name().unwrap(), "-f");
    assert_eq!(
        arg.canonicalize().unwrap(),
        work.path().canonicalize().unwrap().join("-f")
    );
}
