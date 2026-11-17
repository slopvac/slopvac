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
    assert_eq!(text.lines().count(), 233, "232 rules and the count line");
    assert!(
        text.ends_with("232 rule(s) in 26 category(ies)\n"),
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
    assert_eq!(v["rules"].as_array().unwrap().len(), 227);
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
        "https://slopvac.github.io/slopvac/v3/rules/ai-residue/chat-leakage/"
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
        text.contains("Page: https://slopvac.github.io/slopvac/v3/rules/ai-residue/chat-leakage/"),
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
fn rule_output_never_names_a_removed_rule() {
    let removed = [
        "vertical-list-lead-in-missing-colon",
        "vertical-list-item-punctuation",
        "safety-block-missing-consequence",
        "inconsistent-term-for-same-thing",
        "inconsistent-wording-for-same-step",
    ];
    let outputs = [
        stdout(&["explain", "core:ste-descriptive.missing-key-word-structure"]),
        stdout(&["rules", "--format", "md"]),
        stdout(&["rules", "--format", "json"]),
        stdout(&["rules"]),
    ];
    for out in &outputs {
        for id in removed {
            assert!(!out.contains(id), "{id} appears in the output");
        }
    }
}

#[test]
fn the_removed_structure_rules_are_unknown_rules() {
    for id in [
        "core:ste-sentences.vertical-list-lead-in-missing-colon",
        "core:ste-sentences.vertical-list-item-punctuation",
        "core:ste-safety.safety-block-missing-consequence",
        "core:ste-words.inconsistent-term-for-same-thing",
        "core:ste-practices.inconsistent-wording-for-same-step",
    ] {
        slopvac()
            .args(["explain", id])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(format!(
                "error[unknown-rule]: unknown rule: {id}."
            )));
    }
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
    assert_eq!(v["severity_from"], "style preset:google:22:1");
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

/// A base config and a project config that extends it, in one directory.
fn extended(base: &str, project: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("base.toml"),
        format!("schema_version = 3\n\n{base}"),
    )
    .unwrap();
    let file = dir.path().join("slopvac.toml");
    std::fs::write(
        &file,
        format!("schema_version = 3\nextends = [\"base.toml\"]\n\n{project}"),
    )
    .unwrap();
    (dir, file)
}

#[test]
fn a_project_bare_key_beats_an_extended_qualified_key() {
    let (_d, file) = extended(
        "[rules]\n\"core:ai-residue.chat-leakage\" = \"error\"\n",
        "[rules]\n\"ai-residue.chat-leakage\" = \"suggestion\"\n",
    );
    let v = rule_json(&["explain", CHAT_LEAKAGE], &file);
    assert_eq!(v["effective_severity"], "suggestion");
    assert_eq!(v["severity_from"], "project slopvac.toml:5:1");
}

#[test]
fn a_project_qualified_key_beats_an_extended_bare_key() {
    let (_d, file) = extended(
        "[rules]\n\"ai-residue.chat-leakage\" = \"warning\"\n",
        "[rules]\n\"core:ai-residue.chat-leakage\" = \"suggestion\"\n",
    );
    let v = rule_json(&["explain", CHAT_LEAKAGE], &file);
    assert_eq!(v["effective_severity"], "suggestion");
}

#[test]
fn a_qualified_category_key_is_read() {
    let (_d, file) =
        config("schema_version = 3\n\n[categories]\n\"core:ai-residue\" = \"warning\"\n");
    let v = rule_json(&["explain", CHAT_LEAKAGE], &file);
    assert_eq!(v["effective_severity"], "warning");
}

#[test]
fn a_project_category_beats_an_extended_rule_entry_whatever_the_spelling() {
    let (_d, file) = extended(
        "[rules]\n\"core:ai-residue.chat-leakage\" = \"error\"\n",
        "[categories]\n\"core:ai-residue\" = \"suggestion\"\n",
    );
    let v = rule_json(&["explain", CHAT_LEAKAGE], &file);
    assert_eq!(v["effective_severity"], "suggestion");
}

#[test]
fn the_ste_style_lowers_a_rule_through_its_category() {
    // The STE preset sets the ste-punctuation category to suggestion and names no rule in it.
    // The rule ships as a warning.
    let (_d, file) = config("schema_version = 3\nstyles = [\"ste\"]\n");
    let v = rule_json(&["explain", "core:ste-punctuation.semicolon-used"], &file);
    assert_eq!(v["severity"], "warning");
    assert_eq!(v["effective_severity"], "suggestion");
    assert_eq!(v["severity_from"], "style preset:ste:12:21");
}

#[test]
fn a_category_entry_leaves_a_judgement_rule_alone_but_a_rule_entry_sets_it() {
    // concrete-floor is a judgement rule that ships as a suggestion; the orwell category
    // holds checked rules too.
    let (_d, by_category) = config("schema_version = 3\n\n[categories]\norwell = \"error\"\n");
    let v = rule_json(&["explain", JUDGEMENT], &by_category);
    assert_eq!(
        v["effective_severity"], "suggestion",
        "a category does not set it"
    );
    assert!(v.get("severity_from").is_none());
    let checked = rule_json(&["explain", "core:orwell.not-un"], &by_category);
    assert_eq!(
        checked["effective_severity"], "error",
        "checked rules follow the category"
    );

    let (_e, by_rule) = config(
        "schema_version = 3\n\n[categories]\norwell = \"off\"\n\n[rules]\n\"orwell.concrete-floor\" = \"error\"\n",
    );
    let v = rule_json(&["explain", JUDGEMENT], &by_rule);
    assert_eq!(v["effective_severity"], "error");
}

#[test]
fn a_config_diagnostic_never_prints_an_absolute_path_or_source_excerpt() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("slopvac.toml");
    std::fs::write(
        &file,
        "schema_version = 3\nextends = [\"/home/SENTINEL-user-1234/base.toml\"]\n",
    )
    .unwrap();
    let work = tempfile::tempdir().unwrap();
    let out = slopvac()
        .current_dir(work.path())
        .args(["rules", "--config"])
        .arg(&file)
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.starts_with(
            "error[config-extends-missing]: cannot read the extended config `<external>/base.toml`"
        ),
        "{err}"
    );
    assert!(!err.contains("SENTINEL"), "{err}");
    assert!(!err.contains(&dir.path().display().to_string()), "{err}");
    assert!(
        err.contains(" --> <external>/slopvac.toml:2:12") || err.contains("slopvac.toml:2:12"),
        "{err}"
    );
    assert!(
        !err.contains("extends = ["),
        "the source excerpt leaked: {err}"
    );
}

#[test]
fn a_config_path_with_spaces_is_redacted_whole() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("slopvac.toml");
    std::fs::write(
        &file,
        "schema_version = 3\nextends = [\"/home/SENTINEL user 1234/my base.toml\"]\n",
    )
    .unwrap();
    let work = tempfile::tempdir().unwrap();
    let out = slopvac()
        .current_dir(work.path())
        .args(["explain", CHAT_LEAKAGE, "--config"])
        .arg(&file)
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.contains("`<external>/my base.toml`"),
        "the file name is kept: {err}"
    );
    for leaked in ["SENTINEL", "1234", "/home"] {
        assert!(!err.contains(leaked), "{leaked} leaked: {err}");
    }
}

#[test]
fn a_config_path_inside_the_working_directory_is_shown_relative() {
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        work.path().join("slopvac.toml"),
        "schema_version = 3\nextends = [\"conf dir/missing.toml\"]\n",
    )
    .unwrap();
    let out = slopvac()
        .current_dir(work.path())
        .args(["rules", "--config", "slopvac.toml"])
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(err.contains("`conf dir/missing.toml`"), "{err}");
    assert!(err.contains(" --> slopvac.toml:2:12"), "{err}");
    assert!(!err.contains(&work.path().display().to_string()), "{err}");
}

/// Run `rules --config` on a config with the given text, from another directory.
fn refused(text: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("slopvac.toml");
    std::fs::write(&file, text).unwrap();
    let work = tempfile::tempdir().unwrap();
    let out = slopvac()
        .current_dir(work.path())
        .args(["rules", "--config"])
        .arg(&file)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_quoted_absolute_path_in_a_diagnostic_message_is_redacted() {
    // Three producers quote the user's value: a bad enum, a bad key and a bad name.
    for text in [
        "schema_version = 3\nlocale = \"/home/SENTINEL user 1234/secret dir\"\n",
        "schema_version = 3\nprofile = \"/home/SENTINEL user 1234/secret dir\"\n",
        "schema_version = 3\n\"/home/SENTINEL user 1234/secret dir\" = 1\n",
        "schema_version = 3\n\n[rules]\n\"/home/SENTINEL user 1234/secret dir\" = \"off\"\n",
    ] {
        let err = refused(text);
        for leaked in ["SENTINEL", "1234", "/home"] {
            assert!(!err.contains(leaked), "{leaked} leaked for {text:?}: {err}");
        }
        assert!(err.contains("<external>/secret dir"), "{err}");
    }
}

#[test]
fn every_diagnostic_after_the_first_keeps_its_code() {
    let err = refused("schema_version = 3\nlocale = \"klingon\"\nbogus = true\n");
    let mut lines = err.lines().filter(|l| l.starts_with("error["));
    assert!(
        lines
            .next()
            .unwrap()
            .starts_with("error[config-invalid-value]: "),
        "{err}"
    );
    assert!(
        lines
            .next()
            .is_some_and(|l| l.starts_with("error[config-unknown-key]: unknown key `bogus`")),
        "the second diagnostic lost its code: {err}"
    );
}

/// A readable external base file and a project config that extends it, in sibling directories.
fn external_base(base: &str, project_extra: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let outer = tempfile::Builder::new()
        .prefix("SENTINEL-outer")
        .tempdir()
        .unwrap();
    let shared = outer.path().join("shared dir");
    let proj = outer.path().join("proj");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(shared.join("base.toml"), base).unwrap();
    std::fs::write(
        proj.join("slopvac.toml"),
        format!("schema_version = 3\nextends = [\"../shared dir/base.toml\"]\n{project_extra}"),
    )
    .unwrap();
    let file = proj.join("slopvac.toml");
    (outer, file)
}

fn refused_at(file: &std::path::Path) -> String {
    let work = tempfile::tempdir().unwrap();
    let out = slopvac()
        .current_dir(work.path())
        .args(["rules", "--config"])
        .arg(file)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn an_invalid_value_in_a_readable_external_base_hides_its_directories() {
    let (outer, file) = external_base("schema_version = 3\nlocale = \"klingon\"\n", "");
    let err = refused_at(&file);
    assert!(err.starts_with("error[config-invalid-value]: "), "{err}");
    assert!(err.contains(" --> <external>/base.toml:2:10"), "{err}");
    for leaked in [
        "SENTINEL",
        "shared dir",
        &outer.path().display().to_string(),
    ] {
        assert!(!err.contains(leaked), "{leaked} leaked: {err}");
    }
}

#[test]
fn an_extends_cycle_through_an_external_file_hides_its_directories() {
    let (outer, file) = external_base(
        "schema_version = 3\nextends = [\"../proj/slopvac.toml\"]\n",
        "",
    );
    let err = refused_at(&file);
    assert!(err.starts_with("error[config-extends-cycle]: "), "{err}");
    assert!(err.contains("<external>/base.toml"), "{err}");
    for leaked in [
        "SENTINEL",
        "shared dir",
        &outer.path().display().to_string(),
    ] {
        assert!(!err.contains(leaked), "{leaked} leaked: {err}");
    }
}
