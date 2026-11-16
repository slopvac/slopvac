//! `slopvac lint` through the installed binary with the real engine: findings, gate, exit
//! codes, config and the report formats (compat rows `cmd:lint`, `behavior:lint-exit-codes`;
//! docs/contract/lint-report.md).

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

mod support;

/// One line of prose with several slop-lexicon words: 7 errors at the `normal` profile.
const SLOPPY: &str = "# Title\n\nWe leverage a robust, seamless solution to delve into the data. It is important to note that this is a game-changer.\n";
/// Prose with one suggestion-level finding (first person plural) and nothing that gates.
const MILD: &str = "# Title\n\nWe wrote the parser in a week.\n";
const CLEAN: &str = "# Title\n\nThe cat sat on the mat.\n";

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    cmd.current_dir(support::neutral_dir());
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

/// A project directory holding `name` with `text`, and optionally a `slopvac.toml`.
fn project(name: &str, text: &str, config: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    fs::write(&path, text).expect("write document");
    if let Some(config) = config {
        fs::write(dir.path().join("slopvac.toml"), config).expect("write config");
    }
    (dir, path)
}

fn json_report(path: &Path, extra: &[&str]) -> Value {
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .args(extra)
        .arg(path)
        .output()
        .expect("run");
    serde_json::from_slice(&out.stdout).expect("a JSON report")
}

fn rule_ids(report: &Value) -> Vec<String> {
    report["documents"][0]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| f["rule_id"].as_str().expect("rule id").to_owned())
        .collect()
}

#[test]
fn a_lexical_rule_hit_is_reported_at_its_position_and_fails_the_gate() {
    let (_dir, path) = project("bad.md", SLOPPY, None);
    slopvac()
        .arg("lint")
        .arg(&path)
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "3:4 error core:prose-inflation.slop-lexicon",
        ))
        .stdout(predicate::str::contains("FAIL"));
}

#[test]
fn a_clean_file_passes_with_a_full_score() {
    let (_dir, path) = project("ok.md", CLEAN, None);
    slopvac()
        .arg("lint")
        .arg(&path)
        .assert()
        .code(0)
        .stdout(predicate::str::contains("0 findings"))
        .stdout(
            predicate::str::contains("PASS  score 100.0")
                .or(predicate::str::contains("PASS score 100.0")),
        );
}

#[test]
fn findings_that_do_not_exceed_a_threshold_leave_the_exit_code_at_zero() {
    let (_dir, path) = project("mild.md", MILD, None);
    let report = json_report(&path, &[]);
    assert_eq!(rule_ids(&report), ["core:prose-craft.first-person-plural"]);
    assert_eq!(report["documents"][0]["gate"]["passed"], true);
    slopvac().arg("lint").arg(&path).assert().code(0);
}

#[test]
fn stdin_with_a_language_is_linted_under_the_stdin_name() {
    for language in ["markdown", "mdx"] {
        let out = slopvac()
            .args([
                "lint",
                "--stdin",
                "--language",
                language,
                "--format",
                "json",
            ])
            .write_stdin(SLOPPY)
            .assert()
            .code(1)
            .get_output()
            .stdout
            .clone();
        let report: Value = serde_json::from_slice(&out).expect("json");
        assert_eq!(report["documents"][0]["path"], "<stdin>");
        assert!(
            rule_ids(&report).contains(&"core:prose-inflation.slop-lexicon".to_owned()),
            "{language}"
        );
    }
}

#[test]
fn the_json_report_validates_against_the_published_schema() {
    let (_dir, path) = project("bad.md", SLOPPY, None);
    let report = json_report(&path, &[]);
    let validator = support::report_validator();
    let errors: Vec<String> = validator
        .iter_errors(&report)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(report["summary"]["passed"], false);
    assert_eq!(report["summary"]["documents"], 1);
    assert_eq!(report["summary"]["errors"], 7);
}

#[test]
fn sarif_results_carry_the_rule_id_and_a_location() {
    let (_dir, path) = project("bad.md", SLOPPY, None);
    let out = slopvac()
        .args(["lint", "--format", "sarif"])
        .arg(&path)
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let log: Value = serde_json::from_slice(&out).expect("sarif json");
    let results = log["runs"][0]["results"].as_array().expect("results");
    assert_eq!(results.len(), 10);
    let first_error = results
        .iter()
        .find(|r| r["ruleId"] == "core:prose-inflation.slop-lexicon")
        .expect("the slop lexicon result");
    let physical = &first_error["locations"][0]["physicalLocation"];
    assert!(
        physical["artifactLocation"]["uri"]
            .as_str()
            .is_some_and(|u| u.ends_with("bad.md")),
        "{physical}"
    );
    assert_eq!(physical["region"]["startLine"], 3);
    assert_eq!(physical["region"]["startColumn"], 4);
}

#[test]
fn the_out_file_holds_the_real_report() {
    let (dir, path) = project("bad.md", SLOPPY, None);
    let target = dir.path().join("report.json");
    slopvac()
        .args(["lint", "--format", "json", "--out"])
        .arg(&target)
        .arg(&path)
        .assert()
        .code(1);
    let written: Value =
        serde_json::from_str(&fs::read_to_string(&target).expect("report")).unwrap();
    assert_eq!(written["summary"]["findings"], 10);
}

#[test]
fn a_rule_turned_off_in_slopvac_toml_does_not_report() {
    let config = "schema_version = 3\n\n[rules]\n\"core:prose-inflation.slop-lexicon\" = \"off\"\n";
    let (_dir, path) = project("bad.md", SLOPPY, Some(config));
    let ids = rule_ids(&json_report(&path, &[]));
    assert!(
        !ids.contains(&"core:prose-inflation.slop-lexicon".to_owned()),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"core:orwell.unsupported-evaluative".to_owned()),
        "{ids:?}"
    );
}

#[test]
fn a_threshold_in_slopvac_toml_changes_the_gate() {
    let config = "schema_version = 3\n\n[thresholds]\nmax_errors = 100\nmin_score = 0\nmax_total_per_100_words = 1000\n";
    let (_dir, path) = project("bad.md", SLOPPY, Some(config));
    slopvac().arg("lint").arg(&path).assert().code(0);
}

#[test]
fn a_2x_config_lints_nothing_and_points_to_migrate() {
    let (_dir, path) = project("bad.md", SLOPPY, Some("profile = \"normal\"\n"));
    slopvac()
        .arg("lint")
        .arg(&path)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[config-legacy-version]"))
        .stderr(predicate::str::contains("slopvac migrate"));
}

#[test]
fn command_line_options_override_the_files() {
    let (_dir, path) = project("bad.md", SLOPPY, None);
    slopvac()
        .args(["lint", "--min-score", "0", "--max-per-100-words", "1000"])
        .arg(&path)
        .assert()
        .code(1);
    // The error-count threshold still fails the run: only the options given were relaxed.
    let ids = rule_ids(&json_report(&path, &["--disable", "prose-inflation"]));
    assert!(
        !ids.iter().any(|id| id.contains("prose-inflation")),
        "{ids:?}"
    );
    let only = rule_ids(&json_report(&path, &["--category", "orwell"]));
    assert!(
        !only.is_empty() && only.iter().all(|id| id.starts_with("core:orwell.")),
        "{only:?}"
    );
}

#[test]
fn a_file_with_no_engine_parser_is_an_incomplete_run_and_not_skipped() {
    let (dir, _path) = project("ok.md", CLEAN, None);
    let page = dir.path().join("page.html");
    fs::write(&page, "<p>Hello</p>\n").expect("write");
    slopvac()
        .arg("lint")
        .arg(&page)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[input-unsupported]"));
}

#[test]
fn a_missing_target_is_an_incomplete_run() {
    slopvac()
        .args(["lint", "no-such-file.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[input-unreadable]"));
}

#[test]
fn a_directory_target_lints_every_markdown_file_in_it() {
    let (dir, _) = project("a.md", CLEAN, None);
    fs::write(dir.path().join("b.md"), SLOPPY).expect("write");
    let report = json_report(dir.path(), &[]);
    assert_eq!(report["summary"]["documents"], 2);
}

// STUB(slopvac-8l4.12.29): no style pack exists yet, so a config that lists `styles` cannot
// be applied. The run says so instead of passing as if the styles were absent.
#[test]
fn configured_styles_that_cannot_apply_make_the_run_incomplete() {
    let config = "schema_version = 3\nstyles = [\"ste\"]\n";
    let (_dir, path) = project("ok.md", CLEAN, Some(config));
    slopvac()
        .arg("lint")
        .arg(&path)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[styles-not-applied]"));
}
