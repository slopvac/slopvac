//! Installed-binary tests for `slopvac migrate`.
#![cfg(test)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "NO_COLOR",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

fn project(config: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join(".slopvac.toml"), config).expect("write");
    dir
}

#[test]
fn preview_prints_the_diff_and_writes_nothing() {
    let dir = project("profile = \"strict\"\n");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .assert()
        .code(0)
        .stdout(predicate::str::contains("+++ b/slopvac.toml"))
        .stdout(predicate::str::contains("Preview only"));
    assert!(dir.path().join(".slopvac.toml").exists());
    assert!(!dir.path().join("slopvac.toml").exists());
}

#[test]
fn write_migrates_and_restore_undoes_it() {
    let dir = project("profile = \"strict\"\n");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .arg("--write")
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.path().join("slopvac.toml")).expect("migrated"),
        "profile = \"strict\"\nschema_version = 3\n"
    );
    assert!(!dir.path().join(".slopvac.toml").exists());
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .arg("--restore")
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.path().join(".slopvac.toml")).expect("restored"),
        "profile = \"strict\"\n"
    );
    assert!(!dir.path().join("slopvac.toml").exists());
}

#[test]
fn preview_reports_the_findings_the_engine_counts_on_each_side() {
    let dir = project("profile = \"normal\"\n");
    fs::write(
        dir.path().join("doc.md"),
        "<!-- slopvac-allow: rule=orwell.stale-figure reason=dead-metaphor -->\nPick the low-hanging fruit.\n",
    )
    .expect("write");
    // Before, the directive names a 2.x id: the engine reports it and the stale figure.
    // After, `core:orwell.stale-figure` with its listed `dead-metaphor` exception holds.
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .assert()
        .code(0)
        .stdout(predicate::str::contains(
            "Findings by rule (before -> after):\n  core:meta.invalid-suppression: 1 -> 0\n  core:orwell.stale-figure: 1 -> 0\n",
        ));
}

#[test]
fn a_document_the_engine_cannot_lint_exits_2_and_writes_nothing() {
    let dir = project("profile = \"strict\"\n");
    fs::write(dir.path().join("doc.mdx"), "Unclosed {expression\n").expect("write");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .arg("--write")
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[migrate-lint-failed]: "))
        .stderr(predicate::str::contains("doc.mdx"));
    assert!(dir.path().join(".slopvac.toml").exists());
    assert!(!dir.path().join("slopvac.toml").exists());
    assert!(!dir.path().join(".slopvac-migrate-backup").exists());
}

#[test]
fn unrepresentable_setting_exits_2_and_writes_nothing() {
    let dir = project("[vale]\nenabled = true\n");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .arg("--write")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "error[config-legacy-dropped]: .slopvac.toml:2: `vale.enabled`",
        ));
    assert!(dir.path().join(".slopvac.toml").exists());
    assert!(!dir.path().join("slopvac.toml").exists());
}

#[test]
fn restore_rejects_a_manifest_that_escapes_the_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backup = dir.path().join(".slopvac-migrate-backup");
    fs::create_dir_all(&backup).expect("mkdir");
    fs::write(backup.join("manifest.txt"), "C ../x\n").expect("write");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .arg("--restore")
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[migrate-unsafe-path]: "));
}

#[test]
fn missing_project_reports_a_coded_error_without_the_absolute_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no-such-project");
    slopvac()
        .arg("migrate")
        .arg(&missing)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[migrate-io-failed]: "))
        .stderr(predicate::str::contains(dir.path().to_str().expect("utf-8")).not());
}

#[test]
fn conflicting_config_sources_report_a_coded_error() {
    let dir = project("profile = \"strict\"\n");
    fs::write(dir.path().join("slopvac.toml"), "profile = \"normal\"\n").expect("write");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[migrate-config-conflict]: ",
        ))
        .stderr(predicate::str::contains(dir.path().to_str().expect("utf-8")).not());
}

#[test]
fn write_rewrites_a_2x_steering_block_to_the_block_setup_writes() {
    use slopvac_agents::content::BlockKind;
    let text = "# Rules\n\nBe kind.\n\n<!-- slopvac:begin -->\n## Slopvac\n\nOld 2.x text.\n<!-- slopvac:end -->\n\nKeep this.\n";
    let migrated = tempfile::tempdir().expect("tempdir");
    let reference = tempfile::tempdir().expect("tempdir");
    for dir in [&migrated, &reference] {
        fs::write(dir.path().join("AGENTS.md"), text).expect("write");
    }
    slopvac()
        .arg("migrate")
        .arg(migrated.path())
        .arg("--write")
        .assert()
        .code(0);
    slopvac_agents::setup::execute(&slopvac_agents::setup::Options {
        harness: Some("agents".into()),
        list: false,
        remove: false,
        check: false,
        root: reference.path().to_owned(),
        block: BlockKind::Standard,
        no_skill: true,
        agent: false,
        reviewer_model: None,
    })
    .expect("setup");
    assert_eq!(
        fs::read(migrated.path().join("AGENTS.md")).expect("read"),
        fs::read(reference.path().join("AGENTS.md")).expect("read"),
    );
}
