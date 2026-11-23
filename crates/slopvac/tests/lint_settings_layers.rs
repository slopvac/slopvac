//! The layers above the project config reach the settings of each document: its `slopvac:`
//! front-matter block, its inline directives, the genre chain and the packs' styles
//! (`docs/contract/config.md`, section 1.1).
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use serde_json::Value;

mod support;

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    cmd.env("SLOPVAC_NO_UPDATE_CHECK", "1");
    cmd.current_dir(support::neutral_dir());
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

const CONFIG: &str = "schema_version = 3\n\n[terms]\nfiles = [\"terms.toml\"]\n";
const TERMS_FILE: &str =
    "schema_version = 3\n\n[[terms]]\nterm = \"Zorblax\"\nkind = \"product\"\n";
const BODY: &str = "# Title\n\nThe team deploys zorblax on Friday.\n";

/// A project with `config`, a terms file and the document `doc.md`.
fn project(config: &str, doc: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), config).expect("config");
    fs::write(dir.path().join("terms.toml"), TERMS_FILE).expect("terms");
    let path = dir.path().join("doc.md");
    fs::write(&path, doc).expect("doc");
    (dir, path)
}

fn rule_ids(path: &std::path::Path) -> Vec<String> {
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .arg(path)
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("json report");
    report["documents"][0]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| f["rule_id"].as_str().expect("id").to_owned())
        .collect()
}

fn explain(path: &std::path::Path) -> String {
    let out = slopvac()
        .args(["lint", "--explain-config"])
        .arg(path)
        .output()
        .expect("run");
    String::from_utf8(out.stdout).expect("utf-8")
}

#[test]
fn the_front_matter_block_sets_terms_allow_for_its_document() {
    let doc = format!("---\nslopvac:\n  v: 3\n  terms:\n    allow: [zorblax]\n---\n{BODY}");
    let (_dir, path) = project(CONFIG, &doc);
    assert_eq!(rule_ids(&path), Vec::<String>::new());
}

#[test]
fn an_inline_directive_sets_terms_allow_for_its_document() {
    let doc = format!("<!-- slopvac: terms.allow=zorblax -->\n{BODY}");
    let (_dir, path) = project(CONFIG, &doc);
    assert_eq!(rule_ids(&path), Vec::<String>::new());
}

#[test]
fn an_assigned_genre_contributes_its_chain_to_the_config_stack() {
    let config = "schema_version = 3\n\n[[genres]]\nfiles = [\"**\"]\ngenre = \"internal\"\n";
    let (_dir, path) = project(config, BODY);
    let text = explain(&path);
    let line = text
        .lines()
        .find(|l| l.contains("history-internal-reference"))
        .unwrap_or_else(|| panic!("no genre rule level: {text}"));
    assert!(line.contains("genre"), "{line}");
}

#[test]
fn a_pack_qualified_style_loads_from_the_core_pack() {
    let config = "schema_version = 3\nstyles = [\"core:ste\"]\n";
    let (_dir, path) = project(config, BODY);
    let text = explain(&path);
    assert!(text.contains("categories.ste-safety.severity"), "{text}");
}
