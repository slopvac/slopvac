//! The `slopvac-schemas` binary's exit codes: 0 when the committed files match, 1 when one
//! differs, 2 when the check could not run.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::fs;
use std::path::Path;

use rstest::rstest;
use slopvac_schemas::ARTIFACTS;
use snapbox::cmd::{Command, cargo_bin};

/// A scratch repository root holding a copy of the committed generated files.
fn committed_copy() -> tempfile::TempDir {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3");
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("schemas/v3");
    fs::create_dir_all(&dir).unwrap();
    for artifact in ARTIFACTS {
        fs::copy(repo.join(artifact.file), dir.join(artifact.file)).unwrap();
    }
    root
}

fn check(root: &Path) -> Command {
    Command::new(cargo_bin!("slopvac-schemas"))
        .arg("check")
        .arg("--root")
        .arg(root)
}

#[test]
fn check_exits_0_when_the_committed_files_match() {
    let root = committed_copy();
    check(root.path())
        .assert()
        .code(0)
        .stdout_eq("schemas/v3 matches the Rust types\n");
}

#[test]
fn check_exits_1_and_prints_the_diff_when_a_committed_file_was_edited() {
    let root = committed_copy();
    let path = root.path().join("schemas/v3/common.schema.json");
    let text = fs::read_to_string(&path).unwrap();
    let edited = text.replace(
        "\"title\": \"slopvac v3 shared definitions\"",
        "\"title\": \"edited\"",
    );
    assert_ne!(edited, text);
    fs::write(&path, edited).unwrap();
    check(root.path())
        .assert()
        .code(1)
        .stdout_eq(
            "\
--- a/schemas/v3/common.schema.json
+++ b/schemas/v3/common.schema.json
...
-  \"title\": \"edited\"
+  \"title\": \"slopvac v3 shared definitions\"
...
",
        )
        .stderr_eq(
            "1 schema file(s) differ from the Rust types; run `just schemas` and commit the result\n",
        );
}

#[rstest]
#[case::missing(|path: &Path| fs::remove_file(path).unwrap())]
#[case::unreadable(|path: &Path| {
    fs::remove_file(path).unwrap();
    fs::create_dir(path).unwrap();
})]
fn check_exits_2_when_a_committed_file_cannot_be_read(#[case] break_file: fn(&Path)) {
    let root = committed_copy();
    break_file(&root.path().join("schemas/v3/rule.schema.json"));
    check(root.path())
        .assert()
        .code(2)
        .stdout_eq("")
        .stderr_eq("error: cannot access [..]rule.schema.json\n  caused by: [..]\n");
}
