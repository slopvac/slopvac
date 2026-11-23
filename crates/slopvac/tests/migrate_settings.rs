//! `slopvac migrate` lints each side under its own config: the before side under what the
//! 2.x config meant, the after side under the migrated `slopvac.toml`.

use std::error::Error;
use std::fs;
use std::path::Path;

use rstest::rstest;
use slopvac_cli::MigrateConfigure;
use slopvac_cli::rule_catalog;
use slopvac_migrate::{EngineLinter, Finding, Linter, Options, Phase, RenameTable, plan};
use snapbox::cmd::{Command, cargo_bin};

/// One `prose-craft.hyphens` finding: an -ly adverb with a hyphen.
const HYPHEN: &str = "We ship a newly-added flag.\n";
/// One `prose-craft.command-prompt` finding: a shell prompt in a fence.
const PROMPT: &str = "Install it.\n\n```sh\n$ npm install\n```\n";
const HYPHENS: &str = "core:prose-craft.hyphens";
const COMMAND_PROMPT: &str = "core:prose-craft.command-prompt";
/// One `docs-discipline.history-internal-reference` finding under the `reference` genre.
/// The `change-comms` genre turns the rule off.
const HISTORY: &str = "# Title\n\nThe retry loop landed in commit 3f9a2c1 after the outage.\n";
const HISTORY_RULE: &str = "core:docs-discipline.history-internal-reference";

/// A project with the 2.x `config` as `slopvac.toml` and `doc` as `doc.md`.
fn project(config: &str, doc: &str) -> std::io::Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("slopvac.toml"), config)?;
    fs::write(dir.path().join("doc.md"), doc)?;
    Ok(dir)
}

fn count(findings: &[Finding], rule: &str) -> usize {
    findings
        .iter()
        .filter(|f| f.rule == rule)
        .map(|f| f.count)
        .sum()
}

/// The findings of `rule` before and after the migration of `root`.
fn sides(
    root: &Path,
    preserve_behavior: bool,
    rule: &str,
) -> Result<(usize, usize), Box<dyn Error>> {
    let options = Options { preserve_behavior };
    let plan = plan(root, &RenameTable::bundled(), options, "")?;
    let linter = EngineLinter::new(
        vec![rule_catalog::core()?],
        Box::new(MigrateConfigure::load()?),
    );
    let before = linter.lint(root, Phase::Before, &plan.before_overlay(root))?;
    let after = linter.lint(root, Phase::After, &plan.overlay(root))?;
    Ok((count(&before, rule), count(&after, rule)))
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
) -> Result<(), Box<dyn Error>> {
    let dir = project(config, HYPHEN)?;
    assert_eq!(sides(dir.path(), preserve_behavior, HYPHENS)?, expected);
    Ok(())
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
fn the_report_shows_a_gate_the_migration_drops(
    #[case] flags: &[&str],
    #[case] shown: bool,
) -> std::io::Result<()> {
    let dir = project("schema_version = 2\nprofile = \"normal\"\n", PROMPT)?;
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
    Ok(())
}

/// A relaxed 2.x config with an override glob that `--preserve-behavior` cannot place, so its
/// 2.x settings cannot be reproduced for the before side. The block starts on line 4.
const UNREPRODUCIBLE: &str = "schema_version = 2\nprofile = \"relaxed\"\n\n[[overrides]]\n\
                              files = [\"docs/*.md\"]\n\n[overrides.thresholds]\nmax_errors = 0\n";

/// A relaxed 2.x config whose override switches profile and sets a category level, which
/// `--preserve-behavior` refuses, so its 2.x settings cannot be reproduced for the before side
/// either. The block starts on line 4.
const CATEGORY_UNDER_SWITCH: &str = "schema_version = 2\nprofile = \"relaxed\"\n\n\
                                     [[overrides]]\nfiles = [\"docs/**\"]\nprofile = \"normal\"\n\
                                     categories = { ai-tells-agentic = \"warning\" }\n";

/// A relaxed 2.x config with a top-level category level and an override that switches
/// profile, which `--preserve-behavior` refuses. The top-level `categories` table starts on
/// line 4.
const TOP_LEVEL_UNDER_SWITCH: &str = "schema_version = 2\nprofile = \"relaxed\"\n\n\
                                      [categories]\nai-tells-formatting = \"warning\"\n\n\
                                      [[overrides]]\nfiles = [\"docs/**\"]\nprofile = \"normal\"\n";

#[rstest]
#[case::glob_outside_the_grammar(
    UNREPRODUCIBLE,
    "could not be reproduced: unsupported pattern \"docs/*.md\""
)]
#[case::category_under_a_switch(
    CATEGORY_UNDER_SWITCH,
    "could not be reproduced: `overrides[0].categories` on line 4 sets a category that applies above the profile switch in `overrides[0]`"
)]
#[case::top_level_level_under_a_switch(
    TOP_LEVEL_UNDER_SWITCH,
    "could not be reproduced: `categories` on line 4 sets a category that applies above the profile switch in `overrides[0]`"
)]
fn a_config_whose_2x_settings_cannot_be_reproduced_makes_the_comparison_incomplete(
    #[case] config: &str,
    #[case] reason: &str,
) -> std::io::Result<()> {
    let dir = project(config, HYPHEN)?;
    let assert = slopvac().arg("migrate").arg(dir.path()).assert().code(0);
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let notice = stdout
        .lines()
        .find(|l| l.contains("before side for this config is approximate"));
    assert!(
        notice.is_some_and(|l| l.starts_with("warning: slopvac.toml:4: ") && l.contains(reason)),
        "{stdout}"
    );
    assert!(!stdout.contains("same count per rule"), "{stdout}");
    assert!(
        stdout.contains("Findings: the comparison is incomplete for slopvac.toml"),
        "{stdout}"
    );
    Ok(())
}

#[test]
fn a_nested_config_assigns_genres_by_paths_relative_to_its_own_directory()
-> Result<(), Box<dyn Error>> {
    // The 2.x root config gives the migration something to plan. `sub` already holds a 3.0
    // config, which both sides read from disk.
    let dir = project("schema_version = 2\nprofile = \"normal\"\n", "Plain.\n")?;
    fs::create_dir_all(dir.path().join("sub/log"))?;
    fs::create_dir_all(dir.path().join("sub/notes"))?;
    fs::write(
        dir.path().join("sub/slopvac.toml"),
        "schema_version = 3\n\n[[genres]]\nfiles = [\"log/**\"]\ngenre = \"change-comms\"\n",
    )?;
    fs::write(dir.path().join("sub/log/a.md"), HISTORY)?;
    fs::write(dir.path().join("sub/notes/b.md"), HISTORY)?;
    // `sub/log/a.md` is `change-comms`, so only `sub/notes/b.md` reports, on each side.
    assert_eq!(sides(dir.path(), false, HISTORY_RULE)?, (1, 1));
    Ok(())
}
