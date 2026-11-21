//! `slopvac lint` through the installed binary with the real engine: findings, gate, exit
//! codes, config and the report formats (compat rows `cmd:lint`, `behavior:lint-exit-codes`;
//! docs/contract/lint-report.md).
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use assert_cmd::Command;
use predicates::prelude::*;
use rstest::rstest;
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
fn a_config_that_names_an_unknown_category_is_rejected() {
    let config = "schema_version = 3\n\n[categories]\nno-such-category = \"off\"\n";
    let (_dir, path) = project("bad.md", SLOPPY, Some(config));
    slopvac()
        .arg("lint")
        .arg(&path)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[config-unknown-category]"))
        .stderr(predicate::str::contains("slopvac.toml"));
}

const NO_SUCH_RULE: &str = "core:orwell.no-such-rule";

/// The root notices of a JSON report that carry `code`.
fn notices_with<'a>(report: &'a Value, code: &str) -> Vec<&'a Value> {
    report["notices"]
        .as_array()
        .expect("notices")
        .iter()
        .filter(|n| n["code"] == code)
        .collect()
}

/// Lint the clean document of `dir` with its config and return the JSON report. The run must
/// exit 0 and still lint the document.
fn lint_clean_project(dir: &tempfile::TempDir) -> Value {
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .arg(dir.path().join("ok.md"))
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    assert_eq!(report["documents"].as_array().map(Vec::len), Some(1));
    report
}

/// The one `config-unknown-rule` notice of `report`, checked against what the config said.
fn assert_one_unknown_rule_notice(
    dir: &tempfile::TempDir,
    report: &Value,
    rule: &str,
    file: &str,
    line: u64,
    column: u64,
) {
    let found = notices_with(report, "config-unknown-rule");
    assert_eq!(found.len(), 1, "{:?}", report["notices"]);
    let notice = found[0];
    assert_eq!(notice["level"], "warning");
    assert_eq!(notice["path"], file);
    assert_eq!(notice["data"]["rule"], rule);
    assert_eq!(notice["data"]["line"], line);
    assert_eq!(notice["data"]["column"], column);
    let message = notice["message"].as_str().expect("message");
    assert!(message.contains(rule), "{message}");
    assert!(
        message.contains(&format!("{file}:{line}:{column}")),
        "{message}"
    );
    let root = dir.path().to_string_lossy();
    assert!(!message.contains(root.as_ref()), "{message}");
}

#[rstest]
#[case::rules_table(
    "schema_version = 3\n\n[rules]\n\"core:orwell.no-such-rule\" = \"off\"\n",
    None,
    NO_SUCH_RULE,
    "slopvac.toml",
    4
)]
#[case::bare_key(
    "schema_version = 3\n\n[rules]\n\"orwell.no-such-rule\" = \"warning\"\n",
    None,
    "orwell.no-such-rule",
    "slopvac.toml",
    4
)]
#[case::overrides_block(
    "schema_version = 3\n\n[[overrides]]\nfiles = [\"*.md\"]\n\n[overrides.rules]\n\"core:orwell.no-such-rule\" = \"off\"\n",
    None,
    NO_SUCH_RULE,
    "slopvac.toml",
    7
)]
#[case::genres_block(
    "schema_version = 3\n\n[[genres]]\nfiles = [\"*.md\"]\ngenre = \"reference\"\n\n[genres.rules]\n\"core:orwell.no-such-rule\" = \"off\"\n",
    None,
    NO_SUCH_RULE,
    "slopvac.toml",
    8
)]
#[case::extended_config(
    "schema_version = 3\nextends = [\"base.toml\"]\n",
    Some(("base.toml", "schema_version = 3\n\n[rules]\n\"core:orwell.no-such-rule\" = \"off\"\n")),
    NO_SUCH_RULE,
    "base.toml",
    4
)]
fn an_unknown_rule_in_a_config_layer_is_one_warning_notice_and_the_run_still_lints(
    #[case] config: &str,
    #[case] extra: Option<(&str, &str)>,
    #[case] rule: &str,
    #[case] file: &str,
    #[case] line: u64,
) {
    let (dir, _) = project("ok.md", CLEAN, Some(config));
    if let Some((name, text)) = extra {
        fs::write(dir.path().join(name), text).expect("write extended config");
    }
    let report = lint_clean_project(&dir);
    // Each key starts its line, so the notice points at column 1.
    assert_one_unknown_rule_notice(&dir, &report, rule, file, line, 1);
}

#[rstest]
#[case::vertical_list_lead_in("ste-sentences.vertical-list-lead-in-missing-colon")]
#[case::vertical_list_item_punctuation("ste-sentences.vertical-list-item-punctuation")]
#[case::safety_block_consequence("ste-safety.safety-block-missing-consequence")]
#[case::inconsistent_term("ste-words.inconsistent-term-for-same-thing")]
#[case::inconsistent_wording("ste-practices.inconsistent-wording-for-same-step")]
fn a_rule_removed_in_3_0_is_an_unknown_rule_notice_not_an_error(#[case] removed: &str) {
    let config = format!("schema_version = 3\n\n[rules]\n\"{removed}\" = \"off\"\n");
    let (dir, _) = project("ok.md", CLEAN, Some(&config));
    let report = lint_clean_project(&dir);
    assert_one_unknown_rule_notice(&dir, &report, removed, "slopvac.toml", 4, 1);
}

#[test]
fn a_config_with_only_known_rule_ids_has_no_unknown_rule_notice() {
    let config = "schema_version = 3\n\n[rules]\n\"core:orwell.unsupported-evaluative\" = \"off\"\n\"prose-inflation.slop-lexicon\" = \"warning\"\n\n[categories]\norwell = \"warning\"\n\n[[overrides]]\nfiles = [\"*.md\"]\n\n[overrides.rules]\n\"core:prose-inflation.slop-lexicon\" = \"error\"\n";
    let (dir, _) = project("ok.md", CLEAN, Some(config));
    let report = lint_clean_project(&dir);
    assert!(
        notices_with(&report, "config-unknown-rule").is_empty(),
        "{:?}",
        report["notices"]
    );
}

#[test]
fn the_text_report_shows_the_unknown_rule_notice_and_exits_zero() {
    let config = "schema_version = 3\n\n[rules]\n\"core:orwell.no-such-rule\" = \"off\"\n";
    let (_dir, path) = project("ok.md", CLEAN, Some(config));
    slopvac()
        .arg("lint")
        .arg(&path)
        .assert()
        .code(0)
        .stdout(predicate::str::contains(
            "notice[config-unknown-rule]: project slopvac.toml:4:1: unknown rule core:orwell.no-such-rule",
        ));
}

const BASE_WITH_UNKNOWN_RULE: &str =
    "schema_version = 3\n\n[rules]\n\"core:orwell.no-such-rule\" = \"off\"\n";

/// A project that holds `ok.md` and a `slopvac.toml` with `config`, next to a sibling
/// directory `zz-private-dir` that holds `files`. Returns the outer directory and the
/// path of the document.
fn project_beside_private_dir(
    config: &str,
    files: &[(&str, &str)],
) -> (tempfile::TempDir, PathBuf) {
    let outer = tempfile::tempdir().expect("tempdir");
    let proj = outer.path().join("proj");
    let private = outer.path().join("zz-private-dir");
    fs::create_dir_all(&proj).expect("proj");
    fs::create_dir_all(&private).expect("private");
    fs::write(proj.join("ok.md"), CLEAN).expect("document");
    fs::write(proj.join("slopvac.toml"), config).expect("config");
    for (name, text) in files {
        fs::write(private.join(name), text).expect("write extended config");
    }
    let document = proj.join("ok.md");
    (outer, document)
}

#[test]
fn an_unknown_rule_in_an_extended_config_outside_the_root_hides_its_directories() {
    let config = "schema_version = 3\nextends = [\"../zz-private-dir/base.toml\"]\n";
    let (outer, document) =
        project_beside_private_dir(config, &[("base.toml", BASE_WITH_UNKNOWN_RULE)]);
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .arg(&document)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    let found = notices_with(&report, "config-unknown-rule");
    assert_eq!(found.len(), 1, "{:?}", report["notices"]);
    assert_eq!(found[0]["path"], "<external>/base.toml");
    assert_eq!(found[0]["data"]["line"], 4);
    assert_eq!(found[0]["data"]["column"], 1);
    assert_eq!(
        found[0]["message"],
        "extends <external>/base.toml:4:1: unknown rule core:orwell.no-such-rule; \
         use a rule id from `slopvac rules`"
    );
    // The document's `config_sources` keep the fuller provenance form; the notice must not.
    let notice = found[0].to_string();
    for leaked in [
        "zz-private-dir",
        "../",
        outer.path().to_string_lossy().as_ref(),
    ] {
        assert!(!notice.contains(leaked), "{leaked}: {notice}");
    }
}

#[rstest]
#[case::repeated_entry("schema_version = 3\nextends = [\"base.toml\", \"base.toml\"]\n", &[])]
#[case::diamond(
    "schema_version = 3\nextends = [\"left.toml\", \"right.toml\"]\n",
    &[
        ("left.toml", "schema_version = 3\nextends = [\"base.toml\"]\n"),
        ("right.toml", "schema_version = 3\nextends = [\"base.toml\"]\n"),
    ]
)]
fn a_config_reached_twice_through_extends_reports_its_unknown_rule_once(
    #[case] config: &str,
    #[case] extra: &[(&str, &str)],
) {
    let (dir, _) = project("ok.md", CLEAN, Some(config));
    fs::write(dir.path().join("base.toml"), BASE_WITH_UNKNOWN_RULE).expect("base");
    for (name, text) in extra {
        fs::write(dir.path().join(name), text).expect("write extended config");
    }
    let report = lint_clean_project(&dir);
    assert_one_unknown_rule_notice(&dir, &report, NO_SUCH_RULE, "base.toml", 4, 1);
}

#[test]
fn the_same_unknown_rule_at_two_locations_is_two_notices() {
    let config = "schema_version = 3\nextends = [\"left.toml\", \"right.toml\"]\n";
    let (dir, _) = project("ok.md", CLEAN, Some(config));
    fs::write(dir.path().join("left.toml"), BASE_WITH_UNKNOWN_RULE).expect("left");
    fs::write(dir.path().join("right.toml"), BASE_WITH_UNKNOWN_RULE).expect("right");
    let report = lint_clean_project(&dir);
    let paths: Vec<_> = notices_with(&report, "config-unknown-rule")
        .iter()
        .map(|n| n["path"].as_str().expect("path").to_owned())
        .collect();
    assert_eq!(paths, ["left.toml", "right.toml"]);
}

/// The JSON bodies of the LSP frames in `output`.
fn lsp_bodies(output: &str) -> Vec<Value> {
    let mut bodies = Vec::new();
    let mut rest = output;
    while let Some(at) = rest.find("Content-Length: ") {
        let after = &rest[at + "Content-Length: ".len()..];
        let (length, tail) = after.split_once("\r\n\r\n").expect("a frame header");
        let length: usize = length.trim().parse().expect("a length");
        bodies.push(serde_json::from_str(&tail[..length]).expect("a JSON body"));
        rest = &tail[length..];
    }
    bodies
}

/// What a `slopvac lsp` session in `dir` writes for `text` opened as a document.
fn lsp_session(dir: &Path, text: &str) -> Vec<Value> {
    fn frame(body: &str) -> String {
        format!("Content-Length: {}\r\n\r\n{body}", body.len())
    }
    let open = serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {"textDocument": {
            "uri": "file:///work/doc.md", "languageId": "markdown", "version": 1, "text": text}}
    });
    let input: String = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}"#.to_owned(),
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#.to_owned(),
        open.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#.to_owned(),
        r#"{"jsonrpc":"2.0","method":"exit"}"#.to_owned(),
    ]
    .iter()
    .map(|m| frame(m))
    .collect();
    let out = slopvac()
        .current_dir(dir)
        .arg("lsp")
        .write_stdin(input)
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    lsp_bodies(&String::from_utf8_lossy(&out))
}

#[test]
fn the_language_server_warns_once_about_an_unknown_rule_and_keeps_linting() {
    let (dir, _) = project("ok.md", SLOPPY, Some(BASE_WITH_UNKNOWN_RULE));
    let bodies = lsp_session(dir.path(), SLOPPY);
    let shown: Vec<_> = bodies
        .iter()
        .filter(|b| b["method"] == "window/showMessage")
        .collect();
    assert_eq!(shown.len(), 1, "{bodies:?}");
    // LSP message type 2 is Warning.
    assert_eq!(shown[0]["params"]["type"], 2);
    let message = shown[0]["params"]["message"].as_str().expect("message");
    assert!(
        message.contains("unknown rule core:orwell.no-such-rule"),
        "{message}"
    );
    assert!(message.contains("slopvac.toml:4:1"), "{message}");
    let diagnostics = bodies
        .iter()
        .filter(|b| b["method"] == "textDocument/publishDiagnostics")
        .count();
    assert!(diagnostics >= 1, "{bodies:?}");
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

/// The level of every notice in a report, whether it sits on a document or at the root.
fn notice_levels(report: &Value) -> Vec<String> {
    let documents = report["documents"].as_array().expect("documents");
    documents
        .iter()
        .flat_map(|d| d["unchecked"].as_array().into_iter().flatten())
        .chain(report["notices"].as_array().into_iter().flatten())
        .map(|n| n["level"].as_str().expect("level").to_owned())
        .collect()
}

#[rstest::rstest]
#[case::info_notice_keeps_the_run_complete(CLEAN.as_bytes(), "info", 0)]
#[case::unreadable_input_is_incomplete(&[0xff, 0xfe], "warning", 2)]
fn the_exit_code_follows_the_level_of_the_unchecked_notices(
    #[case] contents: &[u8],
    #[case] level: &str,
    #[case] code: i32,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("doc.md");
    fs::write(&path, contents).expect("write document");
    fs::write(dir.path().join("slopvac.toml"), "schema_version = 3\n").expect("write config");
    let report = json_report(&path, &[]);
    assert!(
        notice_levels(&report).iter().any(|l| l == level),
        "no {level} notice in {report}"
    );
    slopvac().arg("lint").arg(&path).assert().code(code);
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
fn a_config_with_an_unknown_rule_is_reported_even_when_no_file_is_left_to_lint() {
    let config = "schema_version = 3\n\n[rules]\n\"core:orwell.nope\" = \"off\"\n";
    let (dir, _) = project("ok.md", CLEAN, Some(config));
    let empty = dir.path().join("empty");
    fs::create_dir(&empty).expect("dir");
    slopvac()
        .arg("lint")
        .arg(&empty)
        .assert()
        .code(0)
        .stdout(predicate::str::contains("notice[config-unknown-rule]"));
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
        .code(0)
        .stdout(predicate::str::contains("notice[config-unknown-rule]"));
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
        .code(0)
        .stdout(predicate::str::contains("notice[config-unknown-rule]"));
}

#[test]
fn an_unwired_command_says_so_without_any_internal_id() {
    let out = slopvac().arg("onboard").output().expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "error[command-not-wired]: `slopvac onboard` is not available in this build yet.\n"
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

/// `shortcut` is a symlink whose target does not exist, so `shortcut/..` cannot be resolved.
/// A fold that assumes it is a plain directory would place the path inside the project.
#[cfg(unix)]
#[test]
fn a_path_through_a_dangling_symlink_hides_the_external_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let top = dir.path().canonicalize().expect("canonical temp dir");
    let base = top.join("base");
    let proj = base.join("proj");
    fs::create_dir_all(&proj).expect("proj dir");
    std::os::unix::fs::symlink(top.join("nowhere"), base.join("shortcut")).expect("symlink");
    fs::write(proj.join("ok.md"), CLEAN).expect("document");

    let via_shortcut = base
        .join("shortcut")
        .join("..")
        .join("proj")
        .join("sentinel-private");
    for format in ["text", "json"] {
        for (is_config, name) in [(true, "missing.toml"), (false, "missing.md")] {
            let target = via_shortcut.join(name);
            let mut cmd = slopvac();
            cmd.current_dir(&proj).args(["lint", "--format", format]);
            if is_config {
                cmd.arg("--config").arg(&target).arg("ok.md");
            } else {
                cmd.arg(&target);
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

/// A flag the build accepts but cannot honour must refuse before linting, not lint quietly.
#[rstest::rstest]
#[case::fix("--fix")]
#[case::diff_working_tree("--diff-working-tree")]
fn an_unwired_lint_flag_refuses_with_exit_2_and_no_report(#[case] flag: &str) {
    let (dir, _) = project("a.md", "It is important to note that this works.\n", None);
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "a.md", flag])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!(
            "error[command-not-wired]: `slopvac lint {flag}` is not available in this build yet.\n"
        )
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

/// A project whose config sets `locale` and overrides it under `docs/`.
const EXPLAIN_CONFIG: &str = "schema_version = 3\nlocale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n";

/// A project with `a.md` and `docs/a.md` under [`EXPLAIN_CONFIG`].
fn explain_project() -> tempfile::TempDir {
    let (dir, _) = project("a.md", SLOPPY, Some(EXPLAIN_CONFIG));
    fs::create_dir(dir.path().join("docs")).expect("docs dir");
    fs::write(dir.path().join("docs/a.md"), SLOPPY).expect("write docs/a.md");
    dir
}

#[test]
fn explain_config_prints_each_value_with_its_source_and_what_it_replaced_and_lints_nothing() {
    let dir = explain_project();
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "--explain-config", "docs/a.md"])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // SLOPPY would fail the gate, so exit 0 shows nothing was linted.
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert!(
        stdout.starts_with("docs/a.md\nconfig root: .\nconfig file: slopvac.toml (project)\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "locale = \"en-US\"  [override slopvac.toml:6:1]\n    replaced \"en-GB\"  [project slopvac.toml:2:1]\n"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("profile.name = \"normal\"  [core]\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("FAIL"), "{stdout}");
    assert!(
        stdout.ends_with("front matter overrides are not shown\n"),
        "{stdout}"
    );
}

#[test]
fn explain_config_gives_each_file_its_own_block_in_argument_order() {
    let dir = explain_project();
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "--explain-config", "a.md", "docs/a.md"])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let (first, second) = stdout
        .split_once("\ndocs/a.md\n")
        .unwrap_or_else(|| panic!("no second block: {stdout}"));
    assert!(first.starts_with("a.md\n"), "{stdout}");
    // Every block ends with the fixed line, and a blank line separates the blocks.
    assert!(
        first.ends_with("front matter overrides are not shown\n"),
        "{stdout}"
    );
    assert!(
        second.ends_with("front matter overrides are not shown\n"),
        "{stdout}"
    );
    assert!(first.contains("locale = \"en-GB\"  [project slopvac.toml:2:1]\n"));
    assert!(!first.contains("override slopvac.toml"), "{stdout}");
    assert!(second.contains("locale = \"en-US\"  [override slopvac.toml:6:1]\n"));
}

#[test]
fn explain_config_names_standard_input_stdin() {
    let dir = explain_project();
    let out = slopvac()
        .current_dir(dir.path())
        .args([
            "lint",
            "--explain-config",
            "--stdin",
            "--language",
            "markdown",
        ])
        .write_stdin(SLOPPY)
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.starts_with("<stdin>\nconfig root: ."), "{stdout}");
}

#[test]
fn explain_config_redacts_a_config_root_above_the_working_directory() {
    let dir = explain_project();
    fs::create_dir(dir.path().join("sub")).expect("sub dir");
    fs::write(dir.path().join("sub/b.md"), SLOPPY).expect("write sub/b.md");
    let out = slopvac()
        .current_dir(dir.path().join("sub"))
        .args(["lint", "--explain-config", "b.md"])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let name = dir.path().file_name().expect("name").to_string_lossy();
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.starts_with(&format!(
            "b.md\nconfig root: <external>/{name}\nconfig file: slopvac.toml (project)\n"
        )),
        "{stdout}"
    );
    let parent = dir.path().parent().expect("parent").to_string_lossy();
    assert!(!stdout.contains(parent.as_ref()), "{stdout}");
}

#[rstest::rstest]
#[case::json("json")]
#[case::sarif("sarif")]
fn explain_config_with_a_report_format_refuses_before_reading_any_file(#[case] format: &str) {
    let out = slopvac()
        .args(["lint", "--explain-config", "--format", format, "missing.md"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!(
            "error[explain-config-format-unsupported]: `--explain-config` prints text only; remove `--format {format}`.\n"
        )
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

/// Options that set a value in a layer `--explain-config` does not show, or that route the
/// report. It refuses each rather than ignoring it.
#[rstest::rstest]
#[case::min_score(&["--min-score", "50"], "--min-score")]
#[case::max_density(&["--max-per-100-words", "5"], "--max-per-100-words")]
#[case::locale(&["--locale", "en-GB"], "--locale")]
#[case::category(&["--category", "prose-craft"], "--category")]
#[case::disable(&["--disable", "prose-craft"], "--disable")]
#[case::out(&["--out", "report.txt"], "--out")]
#[case::open(&["--open"], "--open")]
#[case::verbose(&["--verbose"], "--verbose")]
fn explain_config_refuses_an_option_it_would_ignore(#[case] args: &[&str], #[case] option: &str) {
    let dir = explain_project();
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "--explain-config", "a.md"])
        .args(args)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!(
            "error[explain-config-option-unsupported]: `--explain-config` cannot be used with `{option}`; remove it.\n"
        )
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert!(!dir.path().join("report.txt").exists());
}

#[test]
fn explain_config_refuses_a_format_before_it_reads_slopvac_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), [0xff, 0xfe, 0x00]).expect("write config");
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "--explain-config", "--format", "json", "a.md"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "error[explain-config-format-unsupported]: `--explain-config` prints text only; remove `--format json`.\n"
    );
}

#[cfg(unix)]
#[rstest::rstest]
#[case::escape("a\u{1b}[31m.md", "a\\u{1b}[31m.md")]
#[case::newline("b\nc.md", "b\\u{a}c.md")]
fn explain_config_escapes_a_file_name_in_the_heading(#[case] name: &str, #[case] shown: &str) {
    let dir = explain_project();
    fs::write(dir.path().join(name), SLOPPY).expect("write odd name");
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "--explain-config", "--", name])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.starts_with(&format!("{shown}\nconfig root: .\n")),
        "{stdout:?}"
    );
    assert!(!stdout.contains('\u{1b}'), "{stdout:?}");
}

#[test]
fn explain_config_shows_a_nested_config_root_relative_to_the_working_directory() {
    let dir = explain_project();
    fs::write(
        dir.path().join("docs/slopvac.toml"),
        "schema_version = 3\nlocale = \"en-US\"\n",
    )
    .expect("write nested config");
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "--explain-config", "docs/a.md"])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.starts_with("docs/a.md\nconfig root: docs\nconfig file: slopvac.toml (project)\n"),
        "{stdout}"
    );
}

#[test]
fn a_malformed_argument_is_a_usage_error_before_an_unreadable_config_is_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), [0xff, 0xfe, 0x00]).expect("write config");
    let out = slopvac()
        .current_dir(dir.path())
        .args([
            "lint",
            "--explain-config",
            "--format",
            "json",
            "--min-score",
            "invalid",
            "a.md",
        ])
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.starts_with("error[usage]: "), "{stderr}");
    assert!(!stderr.contains("config-unreadable"), "{stderr}");
}

#[test]
fn explain_config_prints_a_url_value_without_its_user_info_and_query() {
    let (dir, _) = project(
        "a.md",
        SLOPPY,
        Some(
            "schema_version = 3\n[terms]\nallow = [\"https://user:SECRET@host/x?token=SECRET\"]\n",
        ),
    );
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "--explain-config", "a.md"])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(!stdout.contains("SECRET"), "{stdout}");
    assert!(
        stdout.contains("terms.allow = [\"https://host/x\"]"),
        "{stdout}"
    );
}

#[test]
fn explain_config_accepts_the_text_format_given_explicitly() {
    let dir = explain_project();
    slopvac()
        .current_dir(dir.path())
        .args(["lint", "--explain-config", "--format", "text", "a.md"])
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("a.md\nconfig root: .\n"));
}

#[test]
fn diff_base_refuses_with_exit_2_and_no_report() {
    let (dir, _) = project("a.md", "It is important to note that this works.\n", None);
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "a.md", "--diff-base", "main"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "error[command-not-wired]: `slopvac lint --diff-base` is not available in this build yet.\n"
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

/// A project whose config starts an `extends` chain of `links` further files, so 33 links put the
/// chain one level past the 32-level depth limit.
fn extends_chain_project(links: usize) -> (tempfile::TempDir, PathBuf) {
    let (dir, path) = project("doc.md", CLEAN, None);
    let link = |i: usize| format!("link{i}.toml");
    fs::write(
        dir.path().join("slopvac.toml"),
        format!("schema_version = 3\nextends = [\"{}\"]\n", link(1)),
    )
    .expect("write config");
    for i in 1..links {
        fs::write(
            dir.path().join(link(i)),
            format!("schema_version = 3\nextends = [\"{}\"]\n", link(i + 1)),
        )
        .expect("write link");
    }
    fs::write(dir.path().join(link(links)), "schema_version = 3\n").expect("write last link");
    (dir, path)
}

/// A project whose config lists the same leaf file once more than the file limit allows.
fn extends_fan_project() -> (tempfile::TempDir, PathBuf) {
    let (dir, path) = project("doc.md", CLEAN, None);
    fs::write(dir.path().join("leaf.toml"), "schema_version = 3\n").expect("write leaf");
    let entries = vec!["\"leaf.toml\""; slopvac_config::MAX_EXTENDS_FILES + 1].join(", ");
    fs::write(
        dir.path().join("slopvac.toml"),
        format!("schema_version = 3\nextends = [{entries}]\n"),
    )
    .expect("write config");
    (dir, path)
}

/// Assert the notice shape of a refused config and that the report validates.
fn assert_budget_notice(report: &Value, budget: &str, limit: u64, path: &str) {
    let notice = &report["notices"][0];
    assert_eq!(notice["code"], "config-budget-exceeded");
    assert_eq!(notice["data"]["budget"], budget);
    assert_eq!(notice["data"]["limit"], limit);
    assert_eq!(notice["data"]["path"], path);
    let errors: Vec<String> = support::report_validator()
        .iter_errors(report)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

// The path is the file whose `extends` entry tripped the limit. In a chain, slopvac.toml is
// level 0 and link32.toml is level 32: it is the last file accepted, and its own entry would be
// a 33rd level. In the fan, slopvac.toml holds the entry that is the 257th application.
#[rstest::rstest]
#[case::depth(extends_chain_project(33), "extends-depth", 32, "link32.toml")]
#[case::files(extends_fan_project(), "extends-files", 256, "slopvac.toml")]
fn a_config_past_a_budget_reports_it_in_the_json_notice(
    #[case] project: (tempfile::TempDir, PathBuf),
    #[case] budget: &str,
    #[case] limit: u64,
    #[case] path: &str,
) {
    let (_dir, doc) = project;
    assert_budget_notice(&json_report(&doc, &[]), budget, limit, path);
}

#[test]
fn a_budget_tripped_outside_the_project_shows_no_path_of_the_outside_directory() {
    let top = tempfile::tempdir().expect("tempdir");
    let proj = top.path().join("proj");
    let outside = top.path().join("zz-outside-dir");
    fs::create_dir_all(&proj).expect("proj");
    fs::create_dir_all(&outside).expect("outside");
    let doc = proj.join("doc.md");
    fs::write(&doc, CLEAN).expect("doc");
    fs::write(
        proj.join("slopvac.toml"),
        "schema_version = 3\nextends = [\"../zz-outside-dir/link1.toml\"]\n",
    )
    .expect("config");
    for i in 1..33 {
        fs::write(
            outside.join(format!("link{i}.toml")),
            format!("schema_version = 3\nextends = [\"link{}.toml\"]\n", i + 1),
        )
        .expect("link");
    }
    fs::write(outside.join("link33.toml"), "schema_version = 3\n").expect("last");
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .arg(&doc)
        .output()
        .expect("run");
    let text = String::from_utf8(out.stdout).expect("utf8");
    let report: Value = serde_json::from_str(&text).expect("a JSON report");
    assert_budget_notice(&report, "extends-depth", 32, "<external>/link32.toml");
    assert!(!text.contains("zz-outside-dir"), "{text}");
    assert!(
        !text.contains(top.path().to_str().expect("utf8 path")),
        "{text}"
    );
}
