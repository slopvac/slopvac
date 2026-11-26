//! `slopvac migrate --preserve-behavior` keeps the level 2.x gave the word blocklist rule in
//! every file, or refuses the layout before it writes anything. The expected levels come from
//! the `blocklist_2x` section of `vectors.json`, which the 2.x generator measures with the
//! blocklist on. A layout that the migration refuses is named with its reason, and a layout it
//! migrates is checked by linting each file.
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

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn vectors() -> TestResult<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../slopvac-migrate/tests/fixtures/preserve/vectors.json");
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

/// The severity of the blocklist finding in `file` after the migration, or `off` for none.
fn level_3x(root: &Path, file: &str) -> TestResult<String> {
    let out = slopvac()
        .current_dir(root)
        .args(["lint", "--format", "json", file])
        .output()?;
    let report: Value = serde_json::from_slice(&out.stdout)?;
    let findings = report["documents"][0]["findings"]
        .as_array()
        .ok_or("no findings array")?;
    match findings.iter().find(|f| f["rule_id"] == RULE_3X) {
        Some(f) => Ok(f["severity"].as_str().ok_or("no severity")?.to_owned()),
        None => Ok("off".to_owned()),
    }
}

/// A project for the scenario `name`: its 2.x config and one document per measured file.
fn project(scenario: &Value) -> TestResult<(tempfile::TempDir, String)> {
    let input = scenario["input"].as_str().ok_or("no input")?.to_owned();
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    fs::write(root.join("slopvac.toml"), &input)?;
    fs::write(root.join("deploy.toml"), BLOCKLIST)?;
    fs::create_dir(root.join("docs"))?;
    for file in scenario["files"].as_object().ok_or("no files")?.keys() {
        fs::write(root.join(file), "The deploy failed.\n")?;
    }
    Ok((dir, input))
}

fn migrate(root: &Path) -> TestResult<std::process::Output> {
    Ok(slopvac()
        .arg("migrate")
        .arg(root)
        .args(["--write", "--preserve-behavior"])
        .output()?)
}

/// 2.x keeps the level of the rule that an earlier layer sets, and a later category level
/// replaces it in 3.0, or a later category turns on a rule that 2.x left off, or a later
/// category with no `severity` gets the legacy level that outranks the earlier rule. Each
/// layout is refused with its own reason, and nothing is written.
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
// `migrate-reason-blocklist-category-no-severity-over-rule`
#[case::l2_a_later_budget_only_category_over_an_earlier_rule(
    "top-rule-then-an-override-budget-only",
    "with no `severity`, after an earlier layer set a rule of that category"
)]
fn a_layout_the_migration_cannot_keep_is_refused_with_its_own_reason_before_writing(
    #[case] name: &str,
    #[case] reason: &str,
) -> TestResult {
    let data = vectors()?;
    let (dir, input) = project(&data["blocklist_2x"]["scenarios"][name])?;
    let root = dir.path();

    let out = migrate(root)?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{name}: {stderr}");
    assert!(stderr.contains("error[migrate-output-invalid]"), "{stderr}");
    assert!(stderr.contains(reason), "{stderr}");
    assert_eq!(fs::read_to_string(root.join("slopvac.toml"))?, input);
    assert!(!root.join(".slopvac").exists(), "a refusal wrote a backup");
    Ok(())
}

/// A layout that the migration keeps lints at the level 2.x measured in each file.
#[rstest]
#[case::a_later_category_level_over_an_earlier_off_rule("top-rule-off-then-an-override-category")]
fn a_layout_the_migration_keeps_reports_the_2x_level_in_each_file(
    #[case] name: &str,
) -> TestResult {
    let data = vectors()?;
    let blocklist = &data["blocklist_2x"];
    let scenario = &blocklist["scenarios"][name];
    let (dir, _) = project(scenario)?;
    let root = dir.path();

    let out = migrate(root)?;
    assert_eq!(
        out.status.code(),
        Some(0),
        "{name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    for (file, measured) in scenario["files"].as_object().ok_or("no files")? {
        let profile = measured["profile"].as_str().ok_or("no profile")?;
        let want = measured["levels"][RULE_2X]
            .as_str()
            .or_else(|| blocklist["base"][profile]["levels"][RULE_2X].as_str())
            .ok_or("no measured level")?;
        assert_eq!(level_3x(root, file)?, want, "{name}: {file}");
    }
    Ok(())
}
