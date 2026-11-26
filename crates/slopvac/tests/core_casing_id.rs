//! The built-in term-casing rule is a core rule, `core:terms.casing`, and no pack `terms` is
//! built in, so the id `terms` is free for a user plugin (the contract reserves only `core` and
//! the `slopvac` namespace).
//!
//! The CLI does not load synced plugins yet, so these tests do not load a plugin that ships a
//! `terms.casing` rule. That collision check needs the loader and belongs to slopvac-8l4.9.100.

use std::fs;

use snapbox::cmd::{Command, cargo_bin};

fn slopvac(dir: &std::path::Path) -> Command {
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
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir)
}

fn stdout_of(assert: &snapbox::cmd::OutputAssert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
}

#[expect(
    clippy::expect_used,
    reason = "a helper of the tests: a panic is a failed test"
)]
/// A project that declares a plugin with the id `terms`, plus a clean document.
fn project_with_plugins_entry_terms() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("slopvac.toml"),
        "schema_version = 3\n\n[[plugins]]\nid = \"terms\"\nsource = \"plugins/terms\"\n",
    )
    .expect("config");
    fs::write(dir.path().join("doc.md"), "# Title\n\nHello there.\n").expect("doc");
    dir
}

#[test]
fn a_plugins_entry_named_terms_validates_and_lints_without_a_reserved_id_refusal() {
    let dir = project_with_plugins_entry_terms();
    let assert = slopvac(dir.path())
        .args(["lint", "doc.md"])
        .assert()
        .code(0);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("reserved"), "stderr: {stderr}");
}

#[test]
fn the_catalogue_lists_no_plugin_named_terms() {
    let dir = project_with_plugins_entry_terms();
    let assert = slopvac(dir.path())
        .args(["rules", "--format", "json"])
        .assert()
        .code(0);
    let report: serde_json::Value = serde_json::from_str(&stdout_of(&assert)).expect("rules json");
    let plugins: Vec<&str> = report["plugins"]
        .as_array()
        .expect("plugins")
        .iter()
        .map(|p| p.as_str().expect("plugin id"))
        .collect();
    assert_eq!(plugins, ["core"]);
}

#[test]
fn the_catalogue_lists_the_casing_rule_under_core() {
    let dir = project_with_plugins_entry_terms();
    let assert = slopvac(dir.path())
        .args(["rules", "--format", "json"])
        .assert()
        .code(0);
    let report: serde_json::Value = serde_json::from_str(&stdout_of(&assert)).expect("rules json");
    let ids: Vec<&str> = report["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .filter(|id| id.ends_with("terms.casing"))
        .collect();
    assert_eq!(ids, ["core:terms.casing"]);
}
