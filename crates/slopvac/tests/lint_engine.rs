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

const SENTINEL_HOME: &str = "/home/SENTINEL-user-1234";

#[test]
fn a_missing_absolute_config_is_reported_without_its_directories() {
    let out = slopvac()
        .args(["lint", "--format", "json", "--config"])
        .arg(format!("{SENTINEL_HOME}/proj/slopvac.toml"))
        .arg(project("ok.md", CLEAN, None).1)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    let everything = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!everything.contains("SENTINEL"), "{everything}");
    assert!(
        everything.contains("<external>/slopvac.toml"),
        "{everything}"
    );
}

#[test]
fn a_refused_config_value_that_is_a_path_is_redacted_in_stderr_and_notices() {
    let config = format!("schema_version = 3\nlocale = \"{SENTINEL_HOME}/secret dir\"\n");
    let (_dir, path) = project("ok.md", CLEAN, Some(&config));
    for format in ["text", "json", "sarif"] {
        let out = slopvac()
            .args(["lint", "--format", format])
            .arg(&path)
            .output()
            .expect("run");
        assert_eq!(out.status.code(), Some(2), "{format}");
        let everything = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!everything.contains("SENTINEL"), "{format}: {everything}");
    }
}

#[test]
fn a_config_with_an_unknown_rule_is_rejected_even_when_no_file_is_left_to_lint() {
    let config = "schema_version = 3\n\n[rules]\n\"core:orwell.nope\" = \"off\"\n";
    let (dir, _) = project("ok.md", CLEAN, Some(config));
    let empty = dir.path().join("empty");
    fs::create_dir(&empty).expect("dir");
    slopvac()
        .arg("lint")
        .arg(&empty)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[config-unknown-rule]"));
}

#[test]
fn an_excluded_file_is_never_read_so_bad_bytes_in_it_do_not_matter() {
    let config = "schema_version = 3\nexclude = [\"binary.md\"]\n";
    let (dir, path) = project("ok.md", CLEAN, Some(config));
    fs::write(dir.path().join("binary.md"), [0xff, 0xfe, 0x00]).expect("write");
    slopvac()
        .arg("lint")
        .arg(dir.path().join("binary.md"))
        .arg(&path)
        .assert()
        .code(0)
        .stderr(predicate::str::is_empty());
}

#[test]
fn a_local_file_named_like_a_renamed_external_identity_keeps_its_own_name_in_sarif() {
    let (a, _) = outside();
    let (b, _) = outside();
    fs::write(a.path().join("doc.md"), CLEAN).expect("write");
    fs::write(b.path().join("doc.md"), SLOPPY).expect("write");
    // A relative path is shown as given, so this file's identity is literally `<external>/2/doc.md`.
    let work = tempfile::tempdir().expect("tempdir");
    let local = work.path().join("<external>/2");
    fs::create_dir_all(&local).expect("dirs");
    fs::write(local.join("doc.md"), SLOPPY).expect("write");
    let out = slopvac()
        .current_dir(work.path())
        .args(["lint", "--format", "sarif"])
        .arg(a.path().join("doc.md"))
        .arg(b.path().join("doc.md"))
        .arg("<external>/2/doc.md")
        .output()
        .expect("run");
    let log: Value = serde_json::from_slice(&out.stdout).expect("sarif json");
    let mut uris: Vec<String> = log["runs"][0]["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|r| {
            r["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
                .as_str()
                .expect("uri")
                .to_owned()
        })
        .collect();
    uris.sort();
    uris.dedup();
    // The two files that share a name and the local file get three different identities,
    // and no result is lost to a collision.
    assert_eq!(uris.len(), 2, "{uris:?}");
    let report = {
        let json = slopvac()
            .current_dir(work.path())
            .args(["lint", "--format", "json"])
            .arg(a.path().join("doc.md"))
            .arg(b.path().join("doc.md"))
            .arg("<external>/2/doc.md")
            .output()
            .expect("run");
        serde_json::from_slice::<Value>(&json.stdout).expect("json")
    };
    let mut paths: Vec<&str> = report["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .map(|d| d["path"].as_str().expect("path"))
        .collect();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(paths.len(), 3, "{paths:?}");
}

#[test]
fn the_legacy_profile_style_applies_only_when_the_config_lists_it() {
    // `prose-craft.command-prompt` ships as a warning; the 2.x normal levels turn it off.
    let text = "# Title\n\nRun the setup:\n\n```sh\n$ make build\n```\n\nThen use `$ make test` to check.\n";
    let listed_config = "schema_version = 3\nstyles = [\"core:legacy-normal\"]\n";
    let (_a, listed) = project("a.md", text, Some(listed_config));
    let (_b, default) = project("a.md", text, None);
    let without = rule_ids(&json_report(&default, &[]));
    assert_eq!(without, ["core:prose-craft.command-prompt"]);
    let with = rule_ids(&json_report(&listed, &[]));
    assert!(
        !with.iter().any(|id| id.ends_with("command-prompt")),
        "{with:?}"
    );
    slopvac().arg("lint").arg(&listed).assert().code(0);
}

const HISTORY: &str = "# Title\n\nThe retry loop landed in commit 3f9a2c1 after the outage.\n";

fn history_severity(genre: &str, shift: i64) -> Option<String> {
    let config = format!(
        "schema_version = 3\n\n[profile]\nname = \"normal\"\nseverity_shift = {shift}\n\n[[genres]]\nfiles = [\"*.md\"]\ngenre = \"{genre}\"\n"
    );
    let (_dir, path) = project("h.md", HISTORY, Some(&config));
    let report = json_report(&path, &[]);
    report["documents"][0]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|f| f["rule_id"] == "core:docs-discipline.history-internal-reference")
        .map(|f| f["severity"].as_str().expect("severity").to_owned())
}

#[test]
fn a_genre_tolerance_survives_a_severity_shift() {
    // The rule ships as a warning. `change-comms` turns it off, `internal` makes it a
    // suggestion, and `reference` leaves it a warning.
    assert_eq!(history_severity("reference", 0).as_deref(), Some("warning"));
    assert_eq!(
        history_severity("internal", 0).as_deref(),
        Some("suggestion")
    );
    assert_eq!(history_severity("change-comms", 0), None);
    // A shift of +1 moves the genre level one step and leaves `off` off. It does not
    // replace the genre level with the definition's level moved up (an error).
    assert_eq!(history_severity("reference", 1).as_deref(), Some("error"));
    assert_eq!(history_severity("internal", 1).as_deref(), Some("warning"));
    assert_eq!(history_severity("change-comms", 1), None);
}

#[test]
fn a_config_below_a_directory_target_is_checked_even_when_it_excludes_everything() {
    let (dir, _) = project("top.md", CLEAN, None);
    let sub = dir.path().join("sub/deeper");
    fs::create_dir_all(&sub).expect("dirs");
    fs::write(sub.join("a.md"), CLEAN).expect("write");
    fs::write(
        sub.join("slopvac.toml"),
        "schema_version = 3\nexclude = [\"**\"]\n\n[rules]\n\"core:orwell.nope\" = \"off\"\n",
    )
    .expect("config");
    slopvac()
        .arg("lint")
        .arg(dir.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[config-unknown-rule]"));
}

#[test]
fn an_unreadable_absolute_config_is_reported_without_its_directories() {
    let (dir, path) = project("ok.md", CLEAN, None);
    let config = dir.path().join("bad.toml");
    fs::write(&config, [0xff, 0xfe, 0x00]).expect("write");
    let sentinel = dir
        .path()
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned();
    for format in ["text", "json", "sarif"] {
        let out = slopvac()
            .args(["lint", "--format", format, "--config"])
            .arg(&config)
            .arg(&path)
            .output()
            .expect("run");
        assert_eq!(out.status.code(), Some(2), "{format}");
        let everything = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!everything.contains(&sentinel), "{format}: {everything}");
    }
}

#[test]
fn a_config_below_a_glob_target_is_checked_even_when_it_excludes_every_match() {
    let (dir, _) = project("top.md", CLEAN, None);
    let sub = dir.path().join("docs/deeper");
    fs::create_dir_all(&sub).expect("dirs");
    fs::write(sub.join("a.md"), CLEAN).expect("write");
    fs::write(
        sub.join("slopvac.toml"),
        "schema_version = 3\nexclude = [\"**\"]\n\n[rules]\n\"core:orwell.nope\" = \"off\"\n",
    )
    .expect("config");
    slopvac()
        .current_dir(dir.path())
        .args(["lint", "docs/**/*.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[config-unknown-rule]"));
}

#[test]
fn an_unwired_command_says_so_without_any_internal_id() {
    let out = slopvac().arg("packs").output().expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "error[command-not-wired]: `slopvac packs` is not available in this build yet.\n"
    );
}

/// Three dead-opener warnings in a long document: 1.70 severity-weighted findings per 100
/// words, between the `strict` budget (1.5) and the `normal` budget (3.0).
fn three_warnings_in_a_long_document() -> String {
    let paragraph = "The cat sat on the mat. The dog lay by the door.\n\n".repeat(4);
    let opener = "It is important to note that the parser takes a week to write.\n\n".repeat(3);
    format!("# Title\n\n{opener}{paragraph}")
}

const LEGACY_STRICT_STYLE_ONLY: &str = "schema_version = 3\nstyles = [\"core:legacy-strict\"]\n";
const STRICT_PROFILE_AND_STYLE: &str =
    "schema_version = 3\nprofile = \"strict\"\nstyles = [\"core:legacy-strict\"]\n";

#[test]
fn the_strict_profile_with_the_legacy_style_applies_the_strict_density_budget() {
    let (_dir, path) = project(
        "w.md",
        &three_warnings_in_a_long_document(),
        Some(STRICT_PROFILE_AND_STYLE),
    );
    let (code, reasons) = gate_with(&path, &["--min-score", "0"]);
    assert_eq!(code, Some(1));
    assert!(
        reasons
            .iter()
            .any(|r| r == "1.70 severity-weighted findings per 100 words, budget 1.5"),
        "{reasons:?}"
    );
}

#[test]
fn the_legacy_style_alone_keeps_the_normal_density_budget() {
    let (_dir, path) = project(
        "w.md",
        &three_warnings_in_a_long_document(),
        Some(LEGACY_STRICT_STYLE_ONLY),
    );
    let (_, reasons) = gate_with(&path, &["--min-score", "0"]);
    assert!(
        !reasons
            .iter()
            .any(|r| r.starts_with("1.70 severity-weighted")),
        "{reasons:?}"
    );
}

#[test]
fn the_strict_profile_with_the_legacy_style_requires_a_score_of_85() {
    let (_dir, path) = project("bad.md", SLOPPY, Some(STRICT_PROFILE_AND_STYLE));
    let (_, reasons) = gate_with(&path, &[]);
    assert!(
        reasons.contains(&"score 0.0, minimum 85.0".to_owned()),
        "{reasons:?}"
    );
}

#[test]
fn the_legacy_style_alone_requires_a_score_of_70() {
    let (_dir, path) = project("bad.md", SLOPPY, Some(LEGACY_STRICT_STYLE_ONLY));
    let (_, reasons) = gate_with(&path, &[]);
    assert!(
        reasons.contains(&"score 0.0, minimum 70.0".to_owned()),
        "{reasons:?}"
    );
}

/// A working directory `proj` beside a sentinel-named directory that stands for a private
/// location outside the project.
fn proj_beside_sentinel() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let proj = dir.path().join("proj");
    fs::create_dir(&proj).expect("proj dir");
    (dir, proj)
}

#[test]
fn a_missing_target_reached_through_dot_dot_hides_the_external_directory() {
    let (_dir, proj) = proj_beside_sentinel();
    for format in ["text", "json"] {
        let out = slopvac()
            .current_dir(&proj)
            .args([
                "lint",
                "--format",
                format,
                "../sentinel-outside-dir/missing.md",
            ])
            .output()
            .expect("run");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(all.contains("<external>/missing.md"), "{format}: {all}");
        assert!(!all.contains("sentinel-outside-dir"), "{format}: {all}");
    }
}

#[test]
fn an_external_config_error_hides_the_external_directory() {
    let (_dir, proj) = proj_beside_sentinel();
    fs::write(proj.join("ok.md"), CLEAN).expect("document");
    for format in ["text", "json"] {
        let out = slopvac()
            .current_dir(&proj)
            .args([
                "lint",
                "--format",
                format,
                "--config",
                "../sentinel-outside-dir/missing.toml",
                "ok.md",
            ])
            .output()
            .expect("run");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(all.contains("<external>/missing.toml"), "{format}: {all}");
        assert!(!all.contains("sentinel-outside-dir"), "{format}: {all}");
    }
}

/// `link` points at `secret/sub`, so `link/..` is `secret`, not the directory that holds
/// `link`. A lexical fold would turn `base/link/../proj/sentinel-private/...` into a path
/// below the project and print the private directory name.
#[cfg(unix)]
#[test]
fn a_path_through_a_symlinked_parent_still_hides_the_external_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The canonical temp dir, so the working directory and the given paths share a prefix.
    let top = dir.path().canonicalize().expect("canonical temp dir");
    let base = top.join("base");
    let proj = base.join("proj");
    let secret_sub = top.join("secret").join("sub");
    fs::create_dir_all(&proj).expect("proj dir");
    fs::create_dir_all(&secret_sub).expect("secret dir");
    std::os::unix::fs::symlink(&secret_sub, base.join("link")).expect("symlink");
    fs::write(proj.join("ok.md"), CLEAN).expect("document");

    let via_link = base
        .join("link")
        .join("..")
        .join("proj")
        .join("sentinel-private");
    let cases: [(&[&str], &str); 2] = [(&["--config"], "missing.toml"), (&[], "missing.md")];
    for format in ["text", "json"] {
        for (flags, name) in cases {
            let target = via_link.join(name);
            let mut cmd = slopvac();
            cmd.current_dir(&proj).args(["lint", "--format", format]);
            if flags.is_empty() {
                cmd.arg(&target);
            } else {
                cmd.args(flags).arg(&target).arg("ok.md");
            }
            let out = cmd.output().expect("run");
            let all = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                all.contains(&format!("<external>/{name}")),
                "{format} {name}: {all}"
            );
            assert!(!all.contains("sentinel-private"), "{format} {name}: {all}");
        }
    }
}
