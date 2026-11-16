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

/// One dead-opener warning (`It is important to note`) in a document long enough for the
/// density gate to apply (60 words or more). By default it passes: score 90 or more, density
/// under the budget.
fn one_warning_in_a_long_document() -> String {
    let paragraph = "The cat sat on the mat. The dog lay by the door.\n\n".repeat(7);
    format!(
        "# Title\n\nIt is important to note that the parser takes a week to write.\n\n{paragraph}"
    )
}

/// The gate of the only document, from `--format json`, and the exit code.
fn gate_with(path: &Path, flags: &[&str]) -> (Option<i32>, Vec<String>) {
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .args(flags)
        .arg(path)
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    let gate = &report["documents"][0]["gate"];
    let reasons = gate["reasons"]
        .as_array()
        .expect("reasons")
        .iter()
        .map(|r| r.as_str().expect("reason").to_owned())
        .collect();
    (out.status.code(), reasons)
}

#[test]
fn the_baseline_gate_of_the_warning_document_passes() {
    let (_dir, path) = project("warn.md", &one_warning_in_a_long_document(), None);
    let report = json_report(&path, &[]);
    assert_eq!(rule_ids(&report), ["core:prose-craft.dead-opener"]);
    assert_eq!(gate_with(&path, &[]), (Some(0), Vec::new()));
}

#[test]
fn min_score_alone_fails_the_gate_with_its_own_reason() {
    let (_dir, path) = project("warn.md", &one_warning_in_a_long_document(), None);
    let (code, reasons) = gate_with(&path, &["--min-score", "100"]);
    assert_eq!(code, Some(1));
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(
        reasons[0].starts_with("score ") && reasons[0].ends_with("minimum 100.0"),
        "{reasons:?}"
    );
}

#[test]
fn max_per_100_words_alone_fails_the_gate_with_its_own_reason() {
    let (_dir, path) = project("warn.md", &one_warning_in_a_long_document(), None);
    let (code, reasons) = gate_with(&path, &["--max-per-100-words", "0.1"]);
    assert_eq!(code, Some(1));
    // The baseline has no reason at all, so this one comes from the option. A lower density
    // budget also lowers the score, so the score reason may follow it.
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("severity-weighted findings per 100 words, budget 0.1")),
        "{reasons:?}"
    );
}

#[test]
fn relaxed_numeric_options_replace_the_profile_thresholds() {
    let (_dir, path) = project("bad.md", SLOPPY, None);
    let (_, reasons) = gate_with(&path, &["--min-score", "0", "--max-per-100-words", "1000"]);
    // Only the error-count limit of the profile is left.
    assert_eq!(reasons, ["7 error(s), limit 0"]);
}

#[test]
fn disable_and_category_select_rules() {
    let (_dir, path) = project("bad.md", SLOPPY, None);
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
fn an_unknown_category_or_rule_name_is_a_usage_error() {
    let (_dir, path) = project("bad.md", SLOPPY, None);
    slopvac()
        .args(["lint", "--disable", "no-such-category"])
        .arg(&path)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[usage]"));
}

#[test]
fn a_config_that_names_an_unknown_rule_or_category_is_rejected() {
    for (config, code) in [
        (
            "schema_version = 3\n\n[rules]\n\"core:orwell.no-such-rule\" = \"off\"\n",
            "error[config-unknown-rule]",
        ),
        (
            "schema_version = 3\n\n[categories]\nno-such-category = \"off\"\n",
            "error[config-unknown-category]",
        ),
    ] {
        let (_dir, path) = project("bad.md", SLOPPY, Some(config));
        slopvac()
            .arg("lint")
            .arg(&path)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(code))
            .stderr(predicate::str::contains("slopvac.toml"));
    }
}

#[test]
fn bare_rule_keys_and_qualified_category_keys_in_config_apply() {
    let config = "schema_version = 3\n\n[rules]\n\"prose-inflation.slop-lexicon\" = \"off\"\n\n[categories]\n\"core:orwell\" = \"off\"\n";
    let (_dir, path) = project("bad.md", SLOPPY, Some(config));
    let ids = rule_ids(&json_report(&path, &[]));
    assert!(!ids.iter().any(|id| id.contains("slop-lexicon")), "{ids:?}");
    assert!(
        !ids.iter().any(|id| id.starts_with("core:orwell.")),
        "{ids:?}"
    );
    assert!(ids.iter().any(|id| id.contains("ste-words")), "{ids:?}");
}

#[test]
fn a_missing_config_file_is_an_unsuccessful_sarif_run() {
    let (dir, path) = project("ok.md", CLEAN, None);
    let out = slopvac()
        .args(["lint", "--format", "sarif", "--config"])
        .arg(dir.path().join("missing.toml"))
        .arg(&path)
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let log: Value = serde_json::from_slice(&out).expect("sarif json");
    let invocation = &log["runs"][0]["invocations"][0];
    assert_eq!(invocation["executionSuccessful"], false);
    let notifications = invocation["toolExecutionNotifications"]
        .as_array()
        .expect("notifications");
    assert_eq!(notifications.len(), 1, "{notifications:?}");
}

#[test]
fn a_run_that_could_not_start_reports_a_notice_in_json() {
    let (dir, path) = project("ok.md", CLEAN, None);
    let out = slopvac()
        .args(["lint", "--format", "json", "--config"])
        .arg(dir.path().join("missing.toml"))
        .arg(&path)
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(report["notices"][0]["level"], "warning");
}

/// A sentinel directory outside the working directory.
fn outside() -> (tempfile::TempDir, String) {
    let dir = tempfile::Builder::new()
        .prefix("sentinel-")
        .tempdir()
        .expect("tempdir");
    let name = dir
        .path()
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned();
    (dir, name)
}

#[test]
fn error_messages_and_notices_never_show_an_absolute_path() {
    let (dir, sentinel) = outside();
    let doc = dir.path().join("notes.md");
    fs::write(&doc, CLEAN).expect("write");
    // A prose file in comment mode is an unsupported source: a typed discovery error.
    let out = slopvac()
        .args(["lint", "--mode", "code-comments", "--format", "json"])
        .arg(&doc)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    let everything = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!everything.contains(&sentinel), "{everything}");
    assert!(everything.contains("<external>/notes.md"), "{everything}");
}

#[test]
fn files_outside_the_project_with_the_same_name_keep_separate_identities() {
    let (a, _) = outside();
    let (b, _) = outside();
    fs::write(a.path().join("doc.md"), CLEAN).expect("write");
    fs::write(b.path().join("doc.md"), SLOPPY).expect("write");
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .arg(a.path().join("doc.md"))
        .arg(b.path().join("doc.md"))
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("json");
    let paths: Vec<&str> = report["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .map(|d| d["path"].as_str().expect("path"))
        .collect();
    assert_eq!(paths, ["<external>/doc.md", "<external>/2/doc.md"]);
    // The text format reads the document's own text for each finding position.
    let text = slopvac()
        .arg("lint")
        .arg(a.path().join("doc.md"))
        .arg(b.path().join("doc.md"))
        .output()
        .expect("run");
    assert!(String::from_utf8_lossy(&text.stdout).contains("<external>/2/doc.md:3:4"));
}

#[test]
fn exclusions_apply_when_the_config_is_named_with_config() {
    let config = "schema_version = 3\nexclude = [\"bad.md\"]\n";
    let (dir, path) = project("bad.md", SLOPPY, Some(config));
    let out = slopvac()
        .args(["lint", "--format", "json", "--config"])
        .arg(dir.path().join("slopvac.toml"))
        .arg(&path)
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(report["summary"]["documents"], 0);
}

/// A repository outside the working directory that ships its own `python3` must not get to
/// run it while an RST file is linted.
#[cfg(unix)]
#[test]
fn a_python_in_the_linted_repository_is_never_run() {
    use std::os::unix::fs::PermissionsExt;
    let repo = tempfile::tempdir().expect("tempdir");
    fs::create_dir(repo.path().join(".git")).expect("git dir");
    let bin = repo.path().join("bin");
    fs::create_dir(&bin).expect("bin");
    let marker = repo.path().join("executed");
    let script = format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display());
    for name in ["python3", "python"] {
        let tool = bin.join(name);
        fs::write(&tool, &script).expect("write tool");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let docs = repo.path().join("docs");
    fs::create_dir(&docs).expect("docs");
    fs::write(docs.join("a.rst"), "Title\n=====\n\nText.\n").expect("write");
    let path = format!("{}:/usr/bin:/bin", bin.display());
    // The run is incomplete without an interpreter: what matters is what did not run.
    slopvac()
        .env("PATH", path)
        .env_remove("SLOPVAC_RST_CONVERTER")
        .arg("lint")
        .arg(docs.join("a.rst"))
        .assert()
        .code(2);
    assert!(!marker.exists(), "the repository's python3 was executed");
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

#[test]
fn a_built_in_style_in_config_applies_and_an_unknown_one_is_an_error() {
    let (_dir, path) = project(
        "ok.md",
        CLEAN,
        Some("schema_version = 3\nstyles = [\"ste\"]\n"),
    );
    slopvac().arg("lint").arg(&path).assert().code(0);
    let (_dir, path) = project(
        "ok.md",
        CLEAN,
        Some("schema_version = 3\nstyles = [\"nope\"]\n"),
    );
    slopvac()
        .arg("lint")
        .arg(&path)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[unknown-style]"));
}
