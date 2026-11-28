//! `slopvac lint` reads plain text and HTML: a `.txt` file and `--language text` as Markdown,
//! as 2.x does, and a `.html` file and `--language html` as a standalone HTML page whose
//! visible text is the prose.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::path::Path;
use std::time::Duration;

use rstest::rstest;
use snapbox::cmd::{Command, cargo_bin};

/// The finding every case below expects: "leverage" is an error-level slop word.
const SLOP: &str = "error core:prose-inflation.slop-lexicon";

fn slopvac(dir: &Path) -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir)
        .timeout(Duration::from_secs(60))
}

/// Run `cmd`, expect `code`, and return (stdout, stderr).
fn run(cmd: Command, code: i32) -> (String, String) {
    let assert = cmd.assert().code(code);
    let out = assert.get_output();
    (
        String::from_utf8(out.stdout.clone()).expect("utf8 stdout"),
        String::from_utf8(out.stderr.clone()).expect("utf8 stderr"),
    )
}

#[rstest]
// Read as Markdown: the heading is line 1, and `We ` puts the word at column 4 of line 3.
#[case::text("notes.txt", "# Notes\n\nWe leverage the data.\n", "3:4")]
// `<p>We ` is 6 bytes, so the word starts at column 7 of line 2.
#[case::html(
    "page.html",
    "<html><body>\n<p>We leverage the data.</p>\n</body></html>\n",
    "2:7"
)]
fn a_file_is_linted_with_its_finding_in_place(
    #[case] name: &str,
    #[case] text: &str,
    #[case] place: &str,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join(name), text).expect("write");
    // The error fails the gate of the default profile, which allows none.
    let (out, err) = run(slopvac(dir.path()).args(["lint", name]), 1);
    assert!(out.contains(&format!("{place} {SLOP}")), "{out}");
    assert!(!err.contains("input-unsupported"), "{err}");
}

#[rstest]
#[case::text("text", "We leverage the data.\n", "1:4")]
#[case::html("html", "<p>We leverage the data.</p>\n", "1:7")]
fn stdin_is_linted_with_its_finding_in_place(
    #[case] language: &str,
    #[case] text: &str,
    #[case] place: &str,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let cmd = slopvac(dir.path())
        .args(["lint", "--stdin", "--language", language])
        .stdin(text);
    let (out, err) = run(cmd, 1);
    assert!(out.contains(&format!("{place} {SLOP}")), "{out}");
    assert!(!err.contains("input-unsupported"), "{err}");
}

#[test]
fn a_fix_in_an_html_file_changes_the_prose_and_leaves_the_markup() {
    // "leverage" has the substitution "use". The attribute values are markup, not prose.
    let dir = tempfile::tempdir().expect("temp dir");
    let page = dir.path().join("page.html");
    std::fs::write(
        &page,
        "<p title=\"leverage\" data-x=leverage>We leverage the parser.</p>\n",
    )
    .expect("write");
    // The error is gone with the word, so the gate passes.
    run(slopvac(dir.path()).args(["lint", "--fix", "page.html"]), 0);
    assert_eq!(
        std::fs::read_to_string(&page).expect("read"),
        "<p title=\"leverage\" data-x=leverage>We use the parser.</p>\n"
    );
}
