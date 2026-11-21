//! `slopvac lint --fix` through the installed binary: safe substitutions are written back to
//! the file, the report describes the file as it is afterwards, and a file the engine will not
//! write stays as it was (compat rows `opt:lint:--fix`, `behavior:fix-safe-substitutions`;
//! policies.md 1.8 and 1.9).

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use rstest::rstest;

mod support;

const CAP: u64 = 64 * 1024 * 1024;
/// "leverage" has the substitution "use". The first finding on "We" has none.
const FIXABLE: &str = "# Title\n\nWe leverage the parser.\n";
/// `FIXABLE` after the one substitution, nothing else changed.
const FIXED: &str = "# Title\n\nWe use the parser.\n";

fn slopvac(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    cmd.current_dir(dir).env("NO_COLOR", "1");
    for var in ["SLOPVAC_OFFLINE", "SLOPVAC_LOG", "FORCE_COLOR"] {
        cmd.env_remove(var);
    }
    cmd
}

/// A project directory holding `a.md` with `text`.
fn project(text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("a.md"), text).expect("write document");
    dir
}

fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_fixable_finding_is_written_and_the_report_describes_the_fixed_file() {
    let dir = project(FIXABLE);
    slopvac(dir.path())
        .args(["lint", "--fix", "a.md"])
        .assert()
        // The error finding on "leverage" is gone with the word, so the gate now passes.
        .code(0)
        .stdout(predicate::str::contains("first-person-plural"))
        .stdout(predicate::str::contains("approved-word-substitution").not())
        .stdout(predicate::str::contains("slop-lexicon").not());
    assert_eq!(fs::read_to_string(dir.path().join("a.md")).unwrap(), FIXED);
    assert_eq!(entries(dir.path()), ["a.md"], "no temporary file is left");
}

#[test]
fn a_gate_that_still_fails_after_the_fix_exits_1() {
    // "robust" is a slop word with no substitution, so it stays and keeps the gate failed.
    let dir = project("# Title\n\nWe leverage a robust parser.\n");
    slopvac(dir.path())
        .args(["lint", "--fix", "a.md"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("robust"));
    assert_eq!(
        fs::read_to_string(dir.path().join("a.md")).unwrap(),
        "# Title\n\nWe use a robust parser.\n"
    );
}

#[rstest]
#[case::nothing_to_fix(FIXED)]
#[case::finding_without_a_replacement("# Title\n\nWe wrote the parser in a week.\n")]
fn a_file_with_no_applicable_fix_is_not_written(#[case] text: &str) {
    let dir = project(text);
    let before = fs::metadata(dir.path().join("a.md"))
        .unwrap()
        .modified()
        .unwrap();
    slopvac(dir.path())
        .args(["lint", "--fix", "a.md"])
        .assert()
        .code(0);
    let after = fs::metadata(dir.path().join("a.md"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(fs::read_to_string(dir.path().join("a.md")).unwrap(), text);
    assert_eq!(before, after, "the file was rewritten");
}

#[test]
fn a_file_over_the_size_cap_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("big.md");
    fs::File::create(&file)
        .and_then(|f| f.set_len(CAP + 1))
        .expect("sparse file");
    slopvac(dir.path())
        .args(["lint", "--fix", "big.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("document-too-large"));
    assert_eq!(fs::metadata(&file).unwrap().len(), CAP + 1);
}

#[test]
fn a_file_outside_the_working_directory_is_refused_and_left_alone() {
    let project_dir = tempfile::tempdir().expect("tempdir");
    let outside = project(FIXABLE);
    let target = outside.path().join("a.md");
    slopvac(project_dir.path())
        .args(["lint", "--fix"])
        .arg(&target)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("fix-outside-root"))
        .stderr(predicate::str::contains(target.to_string_lossy().as_ref()).not());
    assert_eq!(fs::read_to_string(&target).unwrap(), FIXABLE);
}

#[cfg(unix)]
#[test]
fn a_symlink_to_a_file_outside_the_working_directory_is_refused_and_left_alone() {
    let project_dir = tempfile::tempdir().expect("tempdir");
    let outside = project(FIXABLE);
    std::os::unix::fs::symlink(
        outside.path().join("a.md"),
        project_dir.path().join("link.md"),
    )
    .expect("symlink");
    slopvac(project_dir.path())
        .args(["lint", "--fix", "link.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("fix-outside-root"));
    assert_eq!(
        fs::read_to_string(outside.path().join("a.md")).unwrap(),
        FIXABLE
    );
}

#[test]
fn explain_config_never_writes() {
    let dir = project(FIXABLE);
    slopvac(dir.path())
        .args(["lint", "--explain-config", "--fix", "a.md"])
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.path().join("a.md")).unwrap(),
        FIXABLE
    );
}

#[test]
fn the_report_goes_to_out_and_the_fix_is_still_written() {
    let dir = project(FIXABLE);
    slopvac(dir.path())
        .args([
            "lint", "--fix", "--format", "json", "--out", "r.json", "a.md",
        ])
        .assert()
        .code(0);
    assert_eq!(fs::read_to_string(dir.path().join("a.md")).unwrap(), FIXED);
    let report = fs::read_to_string(dir.path().join("r.json")).unwrap();
    assert!(!report.contains("slop-lexicon"), "{report}");
}

#[test]
fn standard_input_is_refused_as_a_usage_error_and_nothing_is_written() {
    let dir = tempfile::tempdir().expect("tempdir");
    slopvac(dir.path())
        .args(["lint", "--fix", "--stdin", "--language", "markdown"])
        .write_stdin(FIXABLE)
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("error[usage]"));
    assert!(entries(dir.path()).is_empty(), "stdin produced a file");
}
