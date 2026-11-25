//! `slopvac migrate --preserve-behavior` keeps the level 2.x gave the word blocklist rule in
//! every file, or refuses the layout before it writes anything. The expected levels come from
//! the `blocklist_2x` section of `vectors.json`, which the 2.x generator measures with the
//! blocklist on. A layout that the migration refuses is named with its reason.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::Value;
use snapbox::cmd::{Command, cargo_bin};

const BLOCKLIST: &str =
    "schema_version = 3\n\n[[entries]]\nword = \"deploy\"\npos = \"noun\"\nreason = \"x\"\n";

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

fn vectors() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../slopvac-migrate/tests/fixtures/preserve/vectors.json");
    serde_json::from_str(&fs::read_to_string(path).expect("vectors.json")).expect("json")
}

/// A project for the scenario `name`: its 2.x config and one document per measured file.
fn project(scenario: &Value) -> (tempfile::TempDir, String) {
    let input = scenario["input"].as_str().expect("input").to_owned();
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    fs::write(root.join("slopvac.toml"), &input).expect("config");
    fs::write(root.join("deploy.toml"), BLOCKLIST).expect("blocklist");
    fs::create_dir(root.join("docs")).expect("docs dir");
    for file in scenario["files"].as_object().expect("files").keys() {
        fs::write(root.join(file), "The deploy failed.\n").expect("doc");
    }
    (dir, input)
}

fn migrate(root: &Path) -> std::process::Output {
    slopvac()
        .arg("migrate")
        .arg(root)
        .args(["--write", "--preserve-behavior"])
        .output()
        .expect("run")
}

/// 2.x keeps the level of the rule that an earlier layer sets, and a later category level
/// replaces it in 3.0, or a later category turns on a rule that 2.x left off. Each layout is
/// refused with its own reason, and nothing is written.
#[rstest]
// `migrate-reason-blocklist-category-over-rule`
#[case::l1_a_later_category_over_an_earlier_rule(
    "top-rule-then-an-override-category",
    "sets the level of a category that holds a word blocklist rule"
)]
// `migrate-reason-blocklist-category-over-off`
#[case::l2_a_later_floor_over_an_off_category(
    "top-category-off-then-an-override-floor",
    "sets a category that an earlier layer turned off, with no `severity`"
)]
fn a_layout_the_migration_cannot_keep_is_refused_with_its_own_reason_before_writing(
    #[case] name: &str,
    #[case] reason: &str,
) {
    let data = vectors();
    let (dir, input) = project(&data["blocklist_2x"]["scenarios"][name]);
    let root = dir.path();

    let out = migrate(root);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{name}: {stderr}");
    assert!(stderr.contains("error[migrate-output-invalid]"), "{stderr}");
    assert!(stderr.contains(reason), "{stderr}");
    assert_eq!(
        fs::read_to_string(root.join("slopvac.toml")).expect("config"),
        input
    );
    assert!(!root.join(".slopvac").exists(), "a refusal wrote a backup");
}
