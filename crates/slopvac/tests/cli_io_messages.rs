//! The CLI's input and output messages: the general codes keep their `{detail}` shape, and the
//! text that reaches the user keeps its published bytes.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

use std::path::Path;
use std::time::Duration;

use slopvac_cli::messages::catalogs;
use slopvac_messages::{Entry, Kind};
use snapbox::cmd::{Command, cargo_bin};

/// The document cap, 64 MiB, written out here so a change to the shared constant is noticed.
const CAP: usize = 64 * 1024 * 1024;

fn entry(code: &str) -> &'static Entry {
    catalogs()
        .into_iter()
        .find(|(name, _)| *name == "cli io")
        .map(|(_, entries)| entries)
        .expect("the `cli io` catalog is registered")
        .iter()
        .find(|e| e.code == code)
        .unwrap_or_else(|| panic!("`{code}` is not registered"))
}

#[rstest::rstest]
#[case::usage("usage")]
#[case::removed_option("removed-option")]
#[case::render_failed("render-failed")]
#[case::output_failed("output-failed")]
#[case::input_unreadable("input-unreadable")]
#[case::input_not_utf8("input-not-utf8")]
fn a_general_code_is_an_error_that_carries_the_detail_of_its_site(#[case] code: &str) {
    let e = entry(code);
    assert_eq!((e.kind, e.template), (Kind::Error, "{detail}"));
}

fn slopvac(dir: &Path) -> Command {
    Command::new(cargo_bin!("slopvac"))
        .env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir)
        .timeout(Duration::from_secs(60))
}

/// Run `cmd`, expect exit 2, and return (stdout, stderr).
fn refused(cmd: Command) -> (String, String) {
    let assert = cmd.assert().code(2);
    let out = assert.get_output();
    (
        String::from_utf8(out.stdout.clone()).expect("utf8 stdout"),
        String::from_utf8(out.stderr.clone()).expect("utf8 stderr"),
    )
}

#[test]
fn stdin_over_the_cap_is_refused_with_the_published_text() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (stdout, stderr) = refused(
        slopvac(dir.path())
            .args(["lint", "--stdin"])
            .stdin(vec![b'a'; CAP + 1]),
    );
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "error[document-too-large]: <stdin>: larger than the 64 MiB document limit\n"
    );
}

#[test]
fn an_out_path_that_is_a_directory_fails_with_the_published_text() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(dir.path().join("report.txt")).expect("directory");
    std::fs::write(dir.path().join("doc.md"), "one two three\n").expect("document");
    let (stdout, stderr) =
        refused(slopvac(dir.path()).args(["lint", "doc.md", "--out", "report.txt"]));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "error[output-failed]: cannot write the report: is a directory\n"
    );
}
