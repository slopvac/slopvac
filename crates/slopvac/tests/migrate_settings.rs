//! `slopvac migrate` lints each side under its own config: the before side under what the
//! 2.x config meant, the after side under the migrated `slopvac.toml`.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::Path;

use rstest::rstest;
use slopvac_cli::migrate_lint::MigrateConfigure;
use slopvac_cli::rule_catalog;
use slopvac_migrate::{EngineLinter, Finding, Linter, Options, Phase, RenameTable, plan};
use snapbox::cmd::{Command, cargo_bin};

/// One `prose-craft.hyphens` finding: an -ly adverb with a hyphen.
const HYPHEN: &str = "We ship a newly-added flag.\n";
/// One `prose-craft.command-prompt` finding: a shell prompt in a fence.
const PROMPT: &str = "Install it.\n\n```sh\n$ npm install\n```\n";
const HYPHENS: &str = "core:prose-craft.hyphens";
const COMMAND_PROMPT: &str = "core:prose-craft.command-prompt";

/// A project with the 2.x `config` as `slopvac.toml` and `doc` as `doc.md`.
fn project(config: &str, doc: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), config).expect("write config");
    fs::write(dir.path().join("doc.md"), doc).expect("write document");
    dir
}

fn count(findings: &[Finding], rule: &str) -> usize {
    findings
        .iter()
        .filter(|f| f.rule == rule)
        .map(|f| f.count)
        .sum()
}

/// The findings of `rule` before and after the migration of `root`.
fn sides(root: &Path, preserve_behavior: bool, rule: &str) -> (usize, usize) {
    let options = Options { preserve_behavior };
    let plan = plan(root, &RenameTable::bundled(), options, "").expect("the migration plans");
    let linter = EngineLinter::new(
        vec![rule_catalog::core().expect("the rules load")],
        Box::new(MigrateConfigure::load().expect("the rules load")),
    );
    let before = linter
        .lint(root, Phase::Before, &plan.before_overlay(root))
        .expect("the before side lints");
    let after = linter
        .lint(root, Phase::After, &plan.overlay(root))
        .expect("the after side lints");
    (count(&before, rule), count(&after, rule))
}

#[rstest]
// A `[rules]` level holds on both sides, with or without the 2.x gates.
#[case::rule_off("schema_version = 2\n\n[rules]\n\"prose-craft.hyphens\" = \"off\"\n", false, (0, 0))]
#[case::rule_off_preserved(
    "schema_version = 2\n\n[rules]\n\"prose-craft.hyphens\" = \"off\"\n",
    true,
    (0, 0)
)]
#[case::rule_on("schema_version = 2\n\n[rules]\n\"prose-craft.hyphens\" = \"warning\"\n", false, (1, 1))]
// The 2.x relaxed profile turns `prose-craft` off. Before, that applies. After, the plain
// migration keeps only the profile name, so the rule reports at its definition level; with
// `--preserve-behavior` the migrated `core:legacy-relaxed` style turns it off again.
#[case::profile("schema_version = 2\nprofile = \"relaxed\"\n", false, (0, 1))]
#[case::profile_preserved("schema_version = 2\nprofile = \"relaxed\"\n", true, (0, 0))]
fn each_side_lints_under_its_own_config(
    #[case] config: &str,
    #[case] preserve_behavior: bool,
    #[case] expected: (usize, usize),
) {
    let dir = project(config, HYPHEN);
    assert_eq!(sides(dir.path(), preserve_behavior, HYPHENS), expected);
}

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
    cmd.env("NO_COLOR", "1")
}

#[rstest]
// The 2.x normal profile turns `prose-craft.command-prompt` off. Without
// `--preserve-behavior` the migration drops that gate, and the report says so.
#[case::dropped(&[], true)]
#[case::kept(&["--preserve-behavior"], false)]
fn the_report_shows_a_gate_the_migration_drops(#[case] flags: &[&str], #[case] shown: bool) {
    let dir = project("schema_version = 2\nprofile = \"normal\"\n", PROMPT);
    let assert = slopvac()
        .arg("migrate")
        .arg(dir.path())
        .args(flags)
        .assert()
        .code(0);
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert_eq!(
        stdout.contains(&format!("  {COMMAND_PROMPT}: 0 -> 1\n")),
        shown,
        "{stdout}"
    );
    assert_eq!(stdout.contains(COMMAND_PROMPT), shown, "{stdout}");
}
