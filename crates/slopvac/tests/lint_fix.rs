//! `slopvac lint --fix` through the installed binary: safe substitutions are written back to
//! the file, the report describes the file as it is afterwards, and a file the engine will not
//! write stays as it was (compat rows `opt:lint:--fix`, `behavior:fix-safe-substitutions`;
//! policies.md 1.8 and 1.9).
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: the shared support module panics on a broken fixture"
)]

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use rstest::rstest;

mod support;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const CAP: u64 = 64 * 1024 * 1024;
/// "leverage" has the substitution "use". The first finding on "We" has none.
const FIXABLE: &str = "# Title\n\nWe leverage the parser.\n";
/// `FIXABLE` after the one substitution, nothing else changed.
const FIXED: &str = "# Title\n\nWe use the parser.\n";

fn slopvac(dir: &std::path::Path) -> Result<Command, Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("slopvac")?;
    cmd.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1");
    for var in ["SLOPVAC_OFFLINE", "SLOPVAC_LOG", "FORCE_COLOR"] {
        cmd.env_remove(var);
    }
    Ok(cmd)
}

/// An empty project directory.
fn empty_project() -> std::io::Result<tempfile::TempDir> {
    tempfile::tempdir_in(support::neutral_dir())
}

/// A project directory holding `a.md` with `text`.
fn project(text: &str) -> std::io::Result<tempfile::TempDir> {
    let dir = empty_project()?;
    fs::write(dir.path().join("a.md"), text)?;
    Ok(dir)
}

fn entries(dir: &std::path::Path) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        names.push(entry?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

#[test]
fn a_fixable_finding_is_written_and_the_report_describes_the_fixed_file() -> TestResult {
    let dir = project(FIXABLE)?;
    slopvac(dir.path())?
        .args(["lint", "--fix", "a.md"])
        .assert()
        // The error finding on "leverage" is gone with the word, so the gate now passes.
        .code(0)
        .stdout(predicate::str::contains("first-person-plural"))
        .stdout(predicate::str::contains("approved-word-substitution").not())
        .stdout(predicate::str::contains("slop-lexicon").not());
    assert_eq!(fs::read_to_string(dir.path().join("a.md"))?, FIXED);
    assert_eq!(entries(dir.path())?, ["a.md"], "no temporary file is left");
    Ok(())
}

#[test]
fn a_gate_that_still_fails_after_the_fix_exits_1() -> TestResult {
    // "robust" is a slop word with no substitution, so it stays and keeps the gate failed.
    let dir = project("# Title\n\nWe leverage a robust parser.\n")?;
    slopvac(dir.path())?
        .args(["lint", "--fix", "a.md"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("robust"));
    assert_eq!(
        fs::read_to_string(dir.path().join("a.md"))?,
        "# Title\n\nWe use a robust parser.\n"
    );
    Ok(())
}

#[rstest]
#[case::nothing_to_fix(FIXED)]
#[case::finding_without_a_replacement("# Title\n\nWe wrote the parser in a week.\n")]
fn a_file_with_no_applicable_fix_is_not_written(#[case] text: &str) -> TestResult {
    let dir = project(text)?;
    let before = fs::metadata(dir.path().join("a.md"))?.modified()?;
    slopvac(dir.path())?
        .args(["lint", "--fix", "a.md"])
        .assert()
        .code(0);
    let after = fs::metadata(dir.path().join("a.md"))?.modified()?;
    assert_eq!(fs::read_to_string(dir.path().join("a.md"))?, text);
    assert_eq!(before, after, "the file was rewritten");
    Ok(())
}

#[test]
fn a_file_over_the_size_cap_is_refused_and_left_alone() -> TestResult {
    let dir = empty_project()?;
    let file = dir.path().join("big.md");
    fs::File::create(&file)?.set_len(CAP + 1)?;
    slopvac(dir.path())?
        .args(["lint", "--fix", "big.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("document-too-large"));
    assert_eq!(fs::metadata(&file)?.len(), CAP + 1);
    Ok(())
}

#[test]
fn a_file_outside_the_working_directory_is_refused_and_left_alone() -> TestResult {
    let project_dir = empty_project()?;
    let outside = project(FIXABLE)?;
    let target = outside.path().join("a.md");
    slopvac(project_dir.path())?
        .args(["lint", "--fix"])
        .arg(&target)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("fix-outside-root"))
        .stderr(predicate::str::contains(target.to_string_lossy().as_ref()).not());
    assert_eq!(fs::read_to_string(&target)?, FIXABLE);
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_symlink_to_a_file_outside_the_working_directory_is_refused_and_left_alone() -> TestResult {
    let project_dir = empty_project()?;
    let outside = project(FIXABLE)?;
    std::os::unix::fs::symlink(
        outside.path().join("a.md"),
        project_dir.path().join("link.md"),
    )?;
    slopvac(project_dir.path())?
        .args(["lint", "--fix", "link.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("fix-outside-root"));
    assert_eq!(fs::read_to_string(outside.path().join("a.md"))?, FIXABLE);
    Ok(())
}

#[test]
fn explain_config_never_writes() -> TestResult {
    let dir = project(FIXABLE)?;
    slopvac(dir.path())?
        .args(["lint", "--explain-config", "--fix", "a.md"])
        .assert()
        .code(0);
    assert_eq!(fs::read_to_string(dir.path().join("a.md"))?, FIXABLE);
    Ok(())
}

#[test]
fn the_report_goes_to_out_and_the_fix_is_still_written() -> TestResult {
    let dir = project(FIXABLE)?;
    slopvac(dir.path())?
        .args([
            "lint", "--fix", "--format", "json", "--out", "r.json", "a.md",
        ])
        .assert()
        .code(0);
    assert_eq!(fs::read_to_string(dir.path().join("a.md"))?, FIXED);
    let report = fs::read_to_string(dir.path().join("r.json"))?;
    assert!(!report.contains("slop-lexicon"), "{report}");
    Ok(())
}

#[test]
fn standard_input_is_refused_as_a_usage_error_and_nothing_is_written() -> TestResult {
    let dir = empty_project()?;
    slopvac(dir.path())?
        .args(["lint", "--fix", "--stdin", "--language", "markdown"])
        .write_stdin(FIXABLE)
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("error[usage]"));
    assert!(entries(dir.path())?.is_empty(), "stdin produced a file");
    Ok(())
}

/// A document that trips the context-tag ceiling: 150000 eight-byte sentences in one
/// paragraph, over the one tag for every 64 bytes the ceiling allows.
fn over_budget() -> String {
    format!("{}\n", vec!["It must."; 150_000].join(" "))
}

#[rstest]
#[case::lint(&["lint"])]
#[case::lint_fix(&["lint", "--fix"])]
fn an_over_budget_document_reports_the_same_code_with_and_without_fix(
    #[case] args: &[&str],
) -> TestResult {
    let dir = project(&over_budget())?;
    slopvac(dir.path())?
        .args(args)
        .arg("a.md")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[context-budget-exceeded]"));
    Ok(())
}

/// 300000 sentences of 3 bytes in one paragraph.
fn too_many_sentences() -> (String, &'static str, usize) {
    let text = format!("{}\n", vec!["Go."; 300_000].join(" "));
    let limit = text.len() / 8 + 65_536;
    (text, "sentences", limit)
}

/// 300000 table cells of one character.
fn too_many_cells() -> (String, &'static str, usize) {
    let row = format!("{}|\n", "|a".repeat(100));
    let rule = format!("{}|\n", "|-".repeat(100));
    let text = format!("{row}{rule}{}", row.repeat(3_000));
    let limit = text.len() / 4 + 65_536;
    (text, "table-cells", limit)
}

/// The same document as `over_budget`, with its expected budget name and limit.
fn too_many_tags() -> (String, &'static str, usize) {
    let text = over_budget();
    let limit = text.len() / 64 + 65_536;
    (text, "tags", limit)
}

#[rstest]
#[case::tags(too_many_tags())]
#[case::sentences(too_many_sentences())]
#[case::table_cells(too_many_cells())]
fn each_refused_budget_is_named_in_the_notice_data(
    #[case] input: (String, &'static str, usize),
) -> TestResult {
    let (text, budget, limit) = input;
    let dir = project(&text)?;
    slopvac(dir.path())?
        .args(["lint", "--format", "json", "--out", "r.json", "a.md"])
        .assert()
        .code(2);
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("r.json"))?)?;
    let notice = &report["notices"][0];
    assert_eq!(notice["code"], "context-budget-exceeded", "{report}");
    assert_eq!(notice["data"]["budget"], budget, "{report}");
    assert_eq!(notice["data"]["limit"], limit, "{report}");
    assert_eq!(notice["data"]["path"], "a.md", "{report}");
    let message = notice["message"].as_str().unwrap_or_default();
    assert!(!message.contains("Go.") && !message.contains("must"), "{message}");
    Ok(())
}
