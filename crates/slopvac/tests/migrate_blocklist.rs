//! `slopvac migrate --preserve-behavior` keeps a 2.x word blocklist working: 2.x ran the
//! vocabulary sweep once `vocabulary.path` was set, so the migrated config must still report
//! the refused words at the level the 2.x profile gave them.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;

use rstest::rstest;
use serde_json::Value;
use snapbox::cmd::{Command, cargo_bin};

const VOCABULARY: &str = "core:ste-words.word-outside-controlled-vocabulary";
const BLOCKLIST: &str = "schema_version = 3\n\n[[entries]]\nword = \"deploy\"\npos = \"noun\"\n\
                         reason = \"say deployment\"\n";
/// One use of the refused word, as a noun.
const REFUSED: &str = "The deploy failed last night.\n";

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

/// The severities that the vocabulary rule reports in `docs/doc.md` of a project with the
/// 2.x `settings` and a blocklist, after `migrate --write` with `flags`.
fn migrated_report(settings: &str, flags: &[&str]) -> Vec<String> {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    let config =
        format!("schema_version = 2\n{settings}\n[vocabulary]\npath = \"blocklist.toml\"\n");
    fs::write(root.join("slopvac.toml"), config).expect("config");
    fs::write(root.join("blocklist.toml"), BLOCKLIST).expect("blocklist");
    fs::create_dir(root.join("docs")).expect("docs dir");
    fs::write(root.join("docs/doc.md"), REFUSED).expect("doc");
    slopvac()
        .arg("migrate")
        .arg(root)
        .arg("--write")
        .args(flags)
        .assert()
        .code(0);
    let out = slopvac()
        .current_dir(root)
        .args(["lint", "--format", "json", "docs/doc.md"])
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("json report");
    report["documents"][0]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule_id"] == VOCABULARY)
        .map(|f| f["severity"].as_str().expect("severity").to_owned())
        .collect()
}

#[rstest]
// The levels below are what 2.x gave the rule: `error` under strict, `suggestion` under
// normal and off under relaxed (the vocabulary levels in
// docs/contract/compat/profiles/mapping.json).
#[case::normal("profile = \"normal\"\n", &["suggestion"])]
#[case::strict("profile = \"strict\"\n", &["error"])]
#[case::relaxed("profile = \"relaxed\"\n", &[])]
// An authored level for the rule wins over the profile.
#[case::rule_raised(
    "[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    &["error"]
)]
// A rule that 2.x set off stays off.
#[case::rule_off(
    "[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"off\"\n",
    &[]
)]
// A block that sets the rule wins for the files it matches.
#[case::block_off(
    "[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules]\n\
     \"ste-words.word-outside-controlled-vocabulary\" = \"off\"\n",
    &[]
)]
// An authored category level applies as in 2.x.
#[case::category_off("[categories]\n\"ste-words\" = \"off\"\n", &[])]
fn a_blocklist_keeps_reporting_after_a_behavior_preserving_migration(
    #[case] settings: &str,
    #[case] expected: &[&str],
) {
    assert_eq!(
        migrated_report(settings, &["--preserve-behavior"]),
        expected
    );
}
