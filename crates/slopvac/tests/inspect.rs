//! Installed-binary tests for `slopvac rules` and `slopvac explain` (compat rows
//! `cmd:rules`, `cmd:explain`, `opt:rules:*`, `opt:explain:*`,
//! `behavior:inspection-filters`, `behavior:inspection-formats`).
//!
//! Expected counts come from the converted 2.x core files under
//! `crates/slopvac-rules/tests/fixtures/core-v3`, counted with `jq`, not from
//! the code under test. The rules are embedded by STUB(slopvac-8l4.12.27).

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1");
    // The repository root holds a 2.x `slopvac.toml`, which a run from inside it would find.
    // From `/` no config is found, so these tests see the rules as shipped.
    cmd.current_dir("/");
    cmd
}

fn stdout(args: &[&str]) -> String {
    let out = slopvac()
        .args(args)
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("utf-8")
}

fn json(args: &[&str]) -> Value {
    serde_json::from_str(&stdout(args)).expect("json output")
}

const CHAT_LEAKAGE: &str = "core:ai-residue.chat-leakage";

#[test]
fn rules_lists_every_core_rule_with_a_count_line() {
    let text = stdout(&["rules"]);
    assert_eq!(text.lines().count(), 237, "236 rules and the count line");
    assert!(
        text.ends_with("236 rule(s) in 26 category(ies)\n"),
        "{text}"
    );
}

#[test]
fn rules_text_row_shows_id_kind_severity_scope_and_source() {
    let text = stdout(&["rules", "--category", "ai-residue"]);
    let row: Vec<&str> = text.lines().next().unwrap().split_whitespace().collect();
    assert_eq!(&row[..4], [CHAT_LEAKAGE, "pattern", "error", "prose"]);
}

#[test]
fn category_and_kind_filters_combine() {
    let text = stdout(&["rules", "--category", "orwell", "--kind", "judgement"]);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].starts_with("core:orwell.concrete-floor "));
    assert_eq!(lines[1], "1 rule(s) in 1 category(ies)");
}

#[test]
fn category_may_repeat() {
    // The data has 1 ai-residue rule and 5 orwell rules.
    let both = json(&[
        "rules",
        "--category",
        "ai-residue",
        "--category",
        "orwell",
        "--format",
        "json",
    ]);
    assert_eq!(both["rules"].as_array().unwrap().len(), 6);
}

#[test]
fn kind_filter_counts_match_the_data() {
    let v = json(&["rules", "--kind", "judgement", "--format", "json"]);
    assert_eq!(v["rules"].as_array().unwrap().len(), 68);
}

#[test]
fn genre_keeps_only_rules_in_categories_recommended_for_it() {
    let v = json(&["rules", "--genre", "consumer", "--format", "json"]);
    assert_eq!(v["rules"].as_array().unwrap().len(), 231);
}

#[test]
fn a_genre_no_category_recommends_lists_nothing() {
    let text = stdout(&["rules", "--genre", "no-such-genre"]);
    assert_eq!(text, "0 rule(s) in 0 category(ies)\n");
}

#[test]
fn rules_json_has_pack_categories_and_linked_rules() {
    let v = json(&["rules", "--format", "json"]);
    assert_eq!(v["pack"], "core");
    assert_eq!(v["categories"].as_array().unwrap().len(), 26);
    let rule = &v["rules"][0];
    assert_eq!(rule["id"], CHAT_LEAKAGE);
    assert_eq!(rule["category"], "ai-residue");
    assert_eq!(rule["kind"], "pattern");
    assert_eq!(rule["severity"], "error");
    assert_eq!(
        rule["url"],
        "https://srobroek.github.io/slopvac/v3/rules/ai-residue/chat-leakage/"
    );
}

#[test]
fn rules_markdown_groups_rules_under_category_headings() {
    let md = stdout(&["rules", "--category", "ai-residue", "--format", "md"]);
    assert!(
        md.starts_with("# Slopvac rules\n\nThis list has 1 rule,"),
        "{md}"
    );
    assert!(
        md.contains(
            "## AI residue (`ai-residue`), recommended for: consumer, internal, change-comms"
        ),
        "{md}"
    );
    assert!(
        md.contains("### `core:ai-residue.chat-leakage`: Delete chat-session leakage"),
        "{md}"
    );
    assert!(md.contains("- Severity: `error`"), "{md}");
}

#[test]
fn explain_text_shows_the_rule_in_full() {
    let text = stdout(&["explain", CHAT_LEAKAGE]);
    assert!(
        text.starts_with("core:ai-residue.chat-leakage -- Delete chat-session leakage\n"),
        "{text}"
    );
    assert!(
        text.contains("kind: pattern   severity: error   scope: prose"),
        "{text}"
    );
    assert!(
        text.contains("<!-- slopvac-allow: rule=core:ai-residue.chat-leakage reason=quotation -->"),
        "{text}"
    );
    assert!(
        text.contains("Page: https://srobroek.github.io/slopvac/v3/rules/ai-residue/chat-leakage/"),
        "{text}"
    );
}

#[test]
fn explain_json_carries_the_rule_and_its_suppression_comment() {
    let v = json(&["explain", CHAT_LEAKAGE, "--format", "json"]);
    assert_eq!(v["id"], CHAT_LEAKAGE);
    assert_eq!(v["match"]["kind"], "pattern");
    assert_eq!(
        v["suppression"],
        "<!-- slopvac-allow: rule=core:ai-residue.chat-leakage reason=quotation -->"
    );
}

#[test]
fn explain_markdown_is_the_block_rules_markdown_prints() {
    let block = stdout(&["explain", CHAT_LEAKAGE, "--format", "md"]);
    let listing = stdout(&["rules", "--category", "ai-residue", "--format", "md"]);
    let inner = block
        .strip_prefix("<!-- slopvac-disable -->\n")
        .and_then(|b| b.strip_suffix("<!-- slopvac-enable -->\n"))
        .unwrap_or_else(|| panic!("explain markdown lacks the lint wrapper: {block}"));
    assert!(
        inner.starts_with("### `core:ai-residue.chat-leakage`"),
        "{inner}"
    );
    assert!(
        listing.contains(inner.trim_end()),
        "listing lacks the block"
    );
}

#[test]
fn rules_markdown_body_sits_inside_one_lint_wrapper() {
    let md = stdout(&["rules", "--category", "ai-residue", "--format", "md"]);
    assert_eq!(md.matches("<!-- slopvac-disable -->").count(), 1, "{md}");
    assert!(md.ends_with("<!-- slopvac-enable -->\n"), "{md}");
    assert!(md.find("<!-- slopvac-disable -->") > md.find("# Slopvac rules"));
}

// The question text is from `orwell.json` (rule `concrete-floor`).
const QUESTION: &str = "Does this paragraph name at least one number, identifier, path, command, or dated event? If the writer possesses a more specific term than the one used, did they use it?";
const JUDGEMENT: &str = "core:orwell.concrete-floor";

#[test]
fn explain_text_shows_a_judgement_rules_question_and_who_applies_it() {
    let text = stdout(&["explain", JUDGEMENT]);
    assert!(
        text.contains(&format!("Decide by asking: {QUESTION}\n")),
        "{text}"
    );
    assert!(
        text.contains("Lint never reports this rule; `slopvac prime review` applies it."),
        "{text}"
    );
}

#[test]
fn explain_markdown_shows_a_judgement_rules_question_not_a_message() {
    let md = stdout(&["explain", JUDGEMENT, "--format", "md"]);
    assert!(md.contains(&format!("- Question: {QUESTION}\n")), "{md}");
    assert!(!md.contains("- Message:"), "{md}");
}

#[test]
fn rules_markdown_shows_a_judgement_rules_question() {
    let md = stdout(&[
        "rules",
        "--category",
        "orwell",
        "--kind",
        "judgement",
        "--format",
        "md",
    ]);
    assert!(md.contains(&format!("- Question: {QUESTION}\n")), "{md}");
}

#[test]
fn unknown_rule_is_an_incomplete_run() {
    slopvac()
        .args(["explain", "core:no.such-rule"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[unknown-rule]: unknown rule: core:no.such-rule.",
        ));
}

#[test]
fn bare_2x_rule_id_points_at_migrate() {
    slopvac()
        .args(["explain", "ai-residue.chat-leakage"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::starts_with("error[unknown-rule]: ")
                .and(predicate::str::contains("slopvac migrate")),
        );
}

#[test]
fn removed_options_point_at_the_replacement() {
    for args in [
        &["rules", "--profile", "strict"][..],
        &["rules", "--rules-dir", "x"][..],
        &["explain", CHAT_LEAKAGE, "--profile", "strict"][..],
        &["explain", CHAT_LEAKAGE, "--rules-dir", "x"][..],
    ] {
        slopvac()
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[removed-option]: --"));
    }
}

#[test]
fn config_must_exist() {
    slopvac()
        .args(["rules", "--config", "/nonexistent/slopvac.toml"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[config-not-found]: "));
}

#[test]
fn a_missing_absolute_config_path_is_redacted() {
    // A real temporary directory gives an absolute path on every platform.
    let outside = tempfile::Builder::new()
        .prefix("SENTINEL-user-4471")
        .tempdir()
        .unwrap();
    let missing = outside.path().join("proj").join("slopvac.toml");
    // Run from another directory, so the path is outside the working directory.
    let work = tempfile::tempdir().unwrap();
    let out = slopvac()
        .current_dir(work.path())
        .args(["rules", "--config"])
        .arg(&missing)
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.contains("--config <external>/slopvac.toml: no such file"),
        "{err}"
    );
    assert!(!err.contains("SENTINEL-user-4471"), "{err}");
}

#[test]
fn config_that_exists_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("slopvac.toml");
    std::fs::write(&file, "schema_version = 3\n").unwrap();
    slopvac()
        .args(["rules", "--category", "ai-residue", "--config"])
        .arg(&file)
        .assert()
        .code(0);
}

#[test]
fn bad_filter_values_are_usage_errors() {
    for args in [
        &["rules", "--kind", "nonsense"][..],
        &["rules", "--format", "yaml"][..],
        &["rules", "--genre", "Not A Genre"][..],
    ] {
        slopvac()
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[usage]: "));
    }
}

/// A config file in a fresh directory.
fn config(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("slopvac.toml");
    std::fs::write(&file, text).unwrap();
    (dir, file)
}

fn rule_json(args: &[&str], file: &std::path::Path) -> Value {
    let mut all = args.to_vec();
    all.extend(["--format", "json", "--config", file.to_str().unwrap()]);
    json(&all)
}

#[test]
fn a_rules_entry_changes_the_effective_severity_and_names_its_line() {
    let (_dir, file) =
        config("schema_version = 3\n\n[rules]\n\"ai-residue.chat-leakage\" = \"off\"\n");
    let v = rule_json(&["explain", CHAT_LEAKAGE], &file);
    assert_eq!(v["severity"], "error", "the shipped level stays visible");
    assert_eq!(v["effective_severity"], "off");
    assert_eq!(v["severity_from"], "project slopvac.toml:4:1");
}

#[test]
fn a_qualified_rule_key_works_too() {
    let (_dir, file) = config(
        "schema_version = 3\n\n[rules]\n\"core:ai-residue.chat-leakage\" = \"suggestion\"\n",
    );
    let v = rule_json(&["explain", CHAT_LEAKAGE], &file);
    assert_eq!(v["effective_severity"], "suggestion");
}

#[test]
fn a_category_entry_sets_every_rule_in_it() {
    let (_dir, file) = config("schema_version = 3\n\n[categories]\norwell = \"suggestion\"\n");
    let v = rule_json(&["rules", "--category", "orwell"], &file);
    let rules = v["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 5);
    assert!(
        rules
            .iter()
            .all(|r| r["effective_severity"] == "suggestion")
    );
}

#[test]
fn a_rule_entry_beats_a_category_entry_in_the_same_file() {
    let (_dir, file) = config(
        "schema_version = 3\n\n[categories]\nai-residue = \"warning\"\n\n[rules]\n\"ai-residue.chat-leakage\" = \"suggestion\"\n",
    );
    let v = rule_json(&["explain", CHAT_LEAKAGE], &file);
    assert_eq!(v["effective_severity"], "suggestion");
}

#[test]
fn a_bundled_style_changes_severities_and_names_the_style() {
    // The google preset turns the semicolon rule off.
    let (_dir, file) = config("schema_version = 3\nstyles = [\"google\"]\n");
    let v = rule_json(&["explain", "core:ste-punctuation.semicolon-used"], &file);
    assert_eq!(v["effective_severity"], "off");
    assert_eq!(v["severity_from"], "style");
}

#[test]
fn a_project_entry_beats_a_style() {
    let (_dir, file) = config(
        "schema_version = 3\nstyles = [\"google\"]\n\n[rules]\n\"ste-punctuation.semicolon-used\" = \"error\"\n",
    );
    let v = rule_json(&["explain", "core:ste-punctuation.semicolon-used"], &file);
    assert_eq!(v["effective_severity"], "error");
    assert_eq!(v["severity_from"], "project slopvac.toml:5:1");
}

#[test]
fn the_profile_severity_shift_moves_rules_that_no_entry_names() {
    // chat-leakage ships as error and cannot go higher; a warning rule moves to error.
    let (_dir, file) =
        config("schema_version = 3\n\n[profile]\nname = \"normal\"\nseverity_shift = -1\n");
    let v = rule_json(&["explain", CHAT_LEAKAGE], &file);
    assert_eq!(v["effective_severity"], "warning");
    let up = config("schema_version = 3\n\n[profile]\nname = \"normal\"\nseverity_shift = 2\n");
    let w = rule_json(&["explain", CHAT_LEAKAGE], &up.1);
    assert_eq!(
        w["effective_severity"], "error",
        "results clamp at the ends"
    );
}

#[test]
fn text_and_markdown_show_the_configured_severity() {
    let (_dir, file) =
        config("schema_version = 3\n\n[rules]\n\"ai-residue.chat-leakage\" = \"off\"\n");
    let path = file.to_str().unwrap();
    let text = stdout(&["explain", CHAT_LEAKAGE, "--config", path]);
    assert!(text.contains("severity: off"), "{text}");
    assert!(
        text.contains("configured: off (project slopvac.toml:4:1); it ships as error"),
        "{text}"
    );
    let md = stdout(&["explain", CHAT_LEAKAGE, "--format", "md", "--config", path]);
    assert!(
        md.contains("- Severity: `error`; configured: `off` (project slopvac.toml:4:1)"),
        "{md}"
    );
    let table = stdout(&["rules", "--category", "ai-residue", "--config", path]);
    let row: Vec<&str> = table.lines().next().unwrap().split_whitespace().collect();
    assert_eq!(&row[..3], [CHAT_LEAKAGE, "pattern", "off"]);
}

#[test]
fn an_unconfigured_run_reports_the_shipped_severity_without_a_source() {
    let v = json(&["explain", CHAT_LEAKAGE, "--format", "json"]);
    assert_eq!(v["effective_severity"], "error");
    assert!(v.get("severity_from").is_none());
}

#[test]
fn the_nearest_config_applies_without_config_flag() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("slopvac.toml"),
        "schema_version = 3\n\n[rules]\n\"ai-residue.chat-leakage\" = \"warning\"\n",
    )
    .unwrap();
    let out = slopvac()
        .current_dir(dir.path())
        .args(["explain", CHAT_LEAKAGE, "--format", "json"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["effective_severity"], "warning");
}

#[test]
fn a_2x_config_is_refused_with_its_code() {
    let (_dir, file) = config("profile = \"strict\"\n");
    slopvac()
        .args(["rules", "--config"])
        .arg(&file)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[config-legacy-version]: this file has no `schema_version`",
        ));
}

#[test]
fn an_unknown_style_is_named_with_the_styles_that_ship() {
    let (_dir, file) = config("schema_version = 3\nstyles = [\"chicago\"]\n");
    slopvac()
        .args(["rules", "--config"])
        .arg(&file)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[unknown-style]: unknown style \"chicago\"; the styles that ship are: ",
        ));
}
