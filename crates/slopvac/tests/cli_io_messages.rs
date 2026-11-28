//! The CLI's input and output messages: the general codes keep their `{detail}` shape, and the
//! text that reaches the user keeps its published bytes.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

use assert_cmd::Command;
use slopvac_cli::messages::catalogs;
use slopvac_messages::{Entry, Kind};

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

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    cmd.env("SLOPVAC_NO_UPDATE_CHECK", "1").env("NO_COLOR", "1");
    cmd
}

#[test]
fn stdin_over_the_cap_is_refused_with_the_published_text() {
    slopvac()
        .args(["lint", "--stdin"])
        .write_stdin(vec![b'a'; CAP + 1])
        .assert()
        .code(2)
        .stdout("")
        .stderr("error[document-too-large]: <stdin>: larger than the 64 MiB document limit\n");
}

#[test]
fn an_out_path_that_is_a_directory_fails_with_the_published_text() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(dir.path().join("report.txt")).expect("directory");
    std::fs::write(dir.path().join("doc.md"), "one two three\n").expect("document");
    slopvac()
        .current_dir(dir.path())
        .args(["lint", "doc.md", "--out", "report.txt"])
        .assert()
        .code(2)
        .stdout("")
        .stderr("error[output-failed]: cannot write the report: is a directory\n");
}
