//! `slopvac review` through the installed binary: help, the dry-run plan, the errors that
//! stop a run, and paths kept out of messages.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;

mod support;

fn slopvac(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    cmd.current_dir(dir);
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

/// A project directory with an original and a revised document that share one sentence and
/// differ in two: three of the four sentences changed, so the change is a rewrite.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir_in(support::neutral_dir()).expect("tempdir");
    fs::write(
        dir.path().join("old.md"),
        "# Cache\n\nThe cache holds 10 entries. It evicts the oldest entry first.\n",
    )
    .expect("old");
    fs::write(
        dir.path().join("new.md"),
        "# Cache\n\nThe cache holds 20 entries. It evicts the least recently used entry first. Reads refresh an entry.\n",
    )
    .expect("new");
    dir
}

#[test]
fn help_lists_the_inputs_and_the_dry_run_option() {
    let dir = project();
    slopvac(dir.path())
        .args(["review", "--help"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("--original <FILE>"))
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--reviewer-model"));
}

#[test]
fn a_dry_run_prints_the_plan_and_the_exact_payload_and_calls_no_provider() {
    let dir = project();
    let out = slopvac(dir.path())
        .args(["review", "new.md", "--original", "old.md", "--dry-run"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let plan: Value = serde_json::from_slice(&out.stdout).expect("a JSON plan");
    assert_eq!(plan["review_type"], "rewrite");
    assert_eq!(plan["uses_provider"], true);
    assert!(plan.get("report").is_none(), "{plan}");
    let items = plan["payload"]["items"].as_array().expect("items");
    let texts: Vec<&str> = items.iter().filter_map(|i| i["text"].as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("holds 10 entries")));
    assert!(texts.iter().any(|t| t.contains("holds 20 entries")));
}

#[test]
fn a_run_that_needs_a_provider_stops_with_exit_2_and_names_the_way_out() {
    let dir = project();
    slopvac(dir.path())
        .args(["review", "new.md", "--original", "old.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[review-provider-missing]"))
        .stderr(predicate::str::contains("--dry-run"));
}

#[test]
fn offline_mode_blocks_the_provider_call() {
    let dir = project();
    slopvac(dir.path())
        .args(["--offline", "review", "new.md", "--original", "old.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[offline-network-needed]"));
}

#[test]
fn an_unreadable_original_is_reported_without_its_directory() {
    let dir = project();
    let out = slopvac(dir.path())
        .args([
            "review",
            "new.md",
            "--original",
            "/no-such-directory-for-review/x.md",
            "--dry-run",
        ])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("<external>/x.md"), "{stderr}");
    assert!(!stderr.contains("no-such-directory-for-review"), "{stderr}");
}

#[test]
fn a_missing_original_option_is_a_usage_error() {
    let dir = project();
    slopvac(dir.path())
        .args(["review", "new.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[usage]"));
}

/// A project whose config names the model that wrote the revision.
fn project_with_author_model(model: &str) -> tempfile::TempDir {
    let dir = project();
    fs::write(
        dir.path().join("slopvac.toml"),
        format!("schema_version = 3\n\n[review]\nauthor_model = \"{model}\"\n"),
    )
    .expect("config");
    dir
}

#[test]
fn a_reviewer_that_is_the_configured_author_model_is_refused() {
    let dir = project_with_author_model("claude-sonnet-5-5");
    slopvac(dir.path())
        .args([
            "review",
            "new.md",
            "--original",
            "old.md",
            "--dry-run",
            "--reviewer-model",
            "claude-sonnet-5-5#high",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[review-same-model]"));
}

#[test]
fn a_different_reviewer_passes_the_author_model_check_in_a_dry_run() {
    let dir = project_with_author_model("claude-sonnet-5-5");
    let out = slopvac(dir.path())
        .args([
            "review",
            "new.md",
            "--original",
            "old.md",
            "--dry-run",
            "--reviewer-model",
            "gpt-5",
        ])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let plan: Value = serde_json::from_slice(&out.stdout).expect("a JSON plan");
    assert_eq!(plan["reviewer_model_checked"], true, "{plan}");
}
