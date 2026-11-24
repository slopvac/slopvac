//! `slopvac migrate --preserve-behavior` keeps the level 2.x gave the word blocklist rule in
//! every file, or refuses the layout before it writes anything. The expected levels come from
//! the `blocklist_2x` section of `vectors.json`, which the 2.x generator measures with the
//! blocklist on.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::Value;
use snapbox::cmd::{Command, cargo_bin};

const RULE_2X: &str = "ste-words.word-outside-controlled-vocabulary";
const RULE_3X: &str = "core:ste-words.word-outside-controlled-vocabulary";
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

/// The severity of the blocklist finding in `file` after the migration, or `off` for none.
fn level_3x(root: &Path, file: &str) -> String {
    let out = slopvac()
        .current_dir(root)
        .args(["lint", "--format", "json", file])
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("json report");
    report["documents"][0]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|f| f["rule_id"] == RULE_3X)
        .map_or_else(
            || "off".to_owned(),
            |f| f["severity"].as_str().expect("severity").to_owned(),
        )
}

#[rstest]
#[case::top_rule_then_an_override_category("top-rule-then-an-override-category")]
#[case::top_category_off_then_an_override_floor("top-category-off-then-an-override-floor")]
fn the_migration_keeps_the_2x_level_in_each_file_or_refuses_it_before_writing(#[case] name: &str) {
    let data = vectors();
    let blocklist = &data["blocklist_2x"];
    let scenario = &blocklist["scenarios"][name];
    let input = scenario["input"].as_str().expect("input");

    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    fs::write(root.join("slopvac.toml"), input).expect("config");
    fs::write(root.join("deploy.toml"), BLOCKLIST).expect("blocklist");
    fs::create_dir(root.join("docs")).expect("docs dir");
    for file in scenario["files"].as_object().expect("files").keys() {
        fs::write(root.join(file), "The deploy failed.\n").expect("doc");
    }

    let out = slopvac()
        .arg("migrate")
        .arg(root)
        .args(["--write", "--preserve-behavior"])
        .output()
        .expect("run");
    if out.status.code() == Some(2) {
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("error[migrate-output-invalid]"), "{stderr}");
        assert_eq!(
            fs::read_to_string(root.join("slopvac.toml")).expect("config"),
            input
        );
        assert!(!root.join(".slopvac").exists(), "a refusal wrote a backup");
        return;
    }
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    for (file, measured) in scenario["files"].as_object().expect("files") {
        let profile = measured["profile"].as_str().expect("profile");
        let want = measured["levels"][RULE_2X]
            .as_str()
            .or_else(|| blocklist["base"][profile]["levels"][RULE_2X].as_str())
            .expect("a measured level");
        assert_eq!(level_3x(root, file), want, "{name}: {file}");
    }
}
