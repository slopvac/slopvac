//! `slopvac migrate --preserve-behavior` keeps a 2.x word blocklist working. 2.x ran the
//! vocabulary sweep once `vocabulary.path` was set, so the migrated config must report the
//! refused words at the level 2.x gave them, in every file, and refuse the layouts it cannot
//! keep.
//!
//! The expected findings below were measured by running the 2.x reference (`slopvac` 2.18.1 with
//! Vale) on the same project. Each project has the top-level blocklist `deploy.toml`, which
//! refuses `deploy`, and the file `launch.toml`, which refuses `launch`. Every document says
//! "The deploy failed. The launch failed.", and one copy sits inside the `docs/**` override and
//! one outside it.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::Value;
use snapbox::cmd::{Command, cargo_bin};

const VOCABULARY: &str = "core:ste-words.word-outside-controlled-vocabulary";
const RULE: &str = "\"ste-words.word-outside-controlled-vocabulary\"";
const DEPLOY: &str =
    "schema_version = 3\n\n[[entries]]\nword = \"deploy\"\npos = \"noun\"\nreason = \"x\"\n";
const LAUNCH: &str =
    "schema_version = 3\n\n[[entries]]\nword = \"launch\"\npos = \"noun\"\nreason = \"x\"\n";
const DOCUMENT: &str = "The deploy failed. The launch failed.\n";
/// The override that changes the blocklist for the files under `docs`.
const OVERRIDE_PATH: &str =
    "\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.vocabulary]\npath = \"launch.toml\"\n";

fn slopvac() -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1").env("SLOPVAC_NO_UPDATE_CHECK", "1")
}

/// A project: the 2.x `config`, two blocklists, and one document inside `docs` and one outside.
fn project(config: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    fs::write(root.join("slopvac.toml"), config).expect("config");
    fs::write(root.join("deploy.toml"), DEPLOY).expect("blocklist");
    fs::write(root.join("launch.toml"), LAUNCH).expect("blocklist");
    fs::create_dir(root.join("docs")).expect("docs dir");
    fs::write(root.join("docs/in.md"), DOCUMENT).expect("doc");
    fs::write(root.join("out.md"), DOCUMENT).expect("doc");
    dir
}

/// The vocabulary findings of `file` as `(severity, word)`, after the migration.
fn findings(root: &Path, file: &str) -> Vec<(String, String)> {
    let out = slopvac()
        .current_dir(root)
        .args(["lint", "--format", "json", file])
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("json report");
    let mut found: Vec<(String, String)> = report["documents"][0]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule_id"] == VOCABULARY)
        .map(|f| {
            (
                f["severity"].as_str().expect("severity").to_owned(),
                f["matched_text"].as_str().expect("word").to_owned(),
            )
        })
        .collect();
    found.sort();
    found
}

fn expected(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(s, w)| ((*s).to_owned(), (*w).to_owned()))
        .collect()
}

/// Migrate with `--preserve-behavior --write`, then lint the file inside `docs` and the one
/// outside it.
fn migrated(config: &str) -> [Vec<(String, String)>; 2] {
    let dir = project(config);
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .args(["--write", "--preserve-behavior"])
        .assert()
        .code(0);
    [
        findings(dir.path(), "docs/in.md"),
        findings(dir.path(), "out.md"),
    ]
}

#[rstest]
// The 2.x levels of the rule: `suggestion` under normal, `error` under strict, off under
// relaxed.
#[case::normal("schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n", &[("suggestion", "deploy")], &[("suggestion", "deploy")])]
#[case::strict("schema_version = 2\nprofile = \"strict\"\n[vocabulary]\npath = \"deploy.toml\"\n", &[("error", "deploy")], &[("error", "deploy")])]
#[case::relaxed("schema_version = 2\nprofile = \"relaxed\"\n[vocabulary]\npath = \"deploy.toml\"\n", &[], &[])]
// An authored level for the rule wins over the profile, and an authored `off` stays off.
#[case::rule_raised(
    &format!("schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n[rules]\n{RULE} = \"error\"\n"),
    &[("error", "deploy")], &[("error", "deploy")]
)]
#[case::rule_off(
    &format!("schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n[rules]\n{RULE} = \"off\"\n"),
    &[], &[]
)]
#[case::category_off(
    "schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n[categories]\nste-words = \"off\"\n",
    &[], &[]
)]
// An override that changes the path: each file gets its own blocklist at the profile level.
#[case::override_changes_path(
    &format!("schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n{OVERRIDE_PATH}"),
    &[("suggestion", "launch")], &[("suggestion", "deploy")]
)]
// Layout 1: an authored top-level rule level survives an override that moves a path, and an
// authored `off` is not replaced by the profile level.
#[case::inherited_rule_level_under_a_moved_path(
    &format!("schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n[rules]\n{RULE} = \"error\"\n{OVERRIDE_PATH}"),
    &[("error", "launch")], &[("error", "deploy")]
)]
#[case::inherited_off_under_a_moved_path(
    &format!("schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n[rules]\n{RULE} = \"off\"\n{OVERRIDE_PATH}"),
    &[], &[]
)]
// Layout 3: a category floor raises the level of the rule, as in 2.x.
#[case::category_floor(
    "schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n[categories.ste-words]\nseverity = \"suggestion\"\nminimum_severity = \"error\"\n",
    &[("error", "deploy")], &[("error", "deploy")]
)]
// Layout 4: a category that the top level turns off and an override turns on again.
#[case::category_off_then_on_in_an_override(
    "schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n[categories]\nste-words = \"off\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"error\"\n",
    &[("error", "deploy")], &[]
)]
fn a_moved_blocklist_reports_at_the_2x_level_in_every_file(
    #[case] config: &str,
    #[case] inside: &[(&str, &str)],
    #[case] outside: &[(&str, &str)],
) {
    assert_eq!(
        migrated(config),
        [expected(inside), expected(outside)],
        "{config}"
    );
}

#[rstest]
// A blocklist that only an override sets.
#[case::path_only_in_an_override(
    &format!("schema_version = 2\n{OVERRIDE_PATH}"),
    "overrides[0].vocabulary.path"
)]
#[case::path_only_in_an_override_with_a_nested_switch(
    &format!("schema_version = 2\n{OVERRIDE_PATH}\n[[overrides]]\nfiles = [\"docs/api/**\"]\nprofile = \"relaxed\"\n"),
    "overrides[0].vocabulary.path"
)]
// A blocklist together with a switch to a profile whose vocabulary level differs.
#[case::switch_to_another_profile(
    "schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nprofile = \"strict\"\n",
    "overrides[0].profile"
)]
#[case::moved_path_with_a_nested_switch(
    &format!("schema_version = 2\n[vocabulary]\npath = \"deploy.toml\"\n{OVERRIDE_PATH}\n[[overrides]]\nfiles = [\"docs/api/**\"]\nprofile = \"relaxed\"\n"),
    "overrides[1].profile"
)]
fn a_layout_the_migration_cannot_keep_is_refused_and_nothing_is_written(
    #[case] config: &str,
    #[case] at: &str,
) {
    let dir = project(config);
    let assert = slopvac()
        .arg("migrate")
        .arg(dir.path())
        .args(["--write", "--preserve-behavior"])
        .assert()
        .code(2);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("error[migrate-output-invalid]"), "{stderr}");
    assert!(stderr.contains(&format!("`{at}`")), "{stderr}");
    assert_eq!(
        fs::read_to_string(dir.path().join("slopvac.toml")).expect("config"),
        config
    );
    assert!(!dir.path().join(".slopvac").exists());
}
