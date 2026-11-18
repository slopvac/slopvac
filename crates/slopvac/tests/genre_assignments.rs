//! `[[genres]]` assignments in `slopvac.toml` reach the engine: each document gets the genre
//! its path selects, and the genre sets the level of `docs-discipline.history-internal-reference`
//! (`docs/contract/config.md`, `docs/contract/packs.md` section 4).

use std::fs;

use assert_cmd::Command;
use serde_json::Value;

mod support;

const HISTORY: &str = "# Notes\n\nWe refactored the parser.\n";

const CONFIG: &str = r#"schema_version = 3

[[genres]]
files = ["guide/**"]
genre = "consumer"

[[genres]]
files = ["notes/**"]
genre = "internal"

[[genres]]
files = ["log/**", "!log/keep/**"]
genre = "change-comms"
"#;

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
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

fn document<'a>(report: &'a Value, suffix: &str) -> &'a Value {
    report["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .find(|d| d["path"].as_str().is_some_and(|p| p.ends_with(suffix)))
        .unwrap_or_else(|| panic!("no document ends with {suffix}: {report}"))
}

fn history_levels(document: &Value) -> Vec<String> {
    document["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule_id"] == "core:docs-discipline.history-internal-reference")
        .map(|f| f["severity"].as_str().expect("severity").to_owned())
        .collect()
}

#[test]
fn genres_blocks_choose_the_genre_and_level_of_each_document() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), CONFIG).expect("write config");
    for rel in ["guide/a.md", "notes/b.md", "log/c.md", "log/keep/d.md"] {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, HISTORY).expect("write document");
    }
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .arg(dir.path())
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");

    // (document, genre, source of the choice, levels of the history rule)
    let expected: [(&str, &str, &str, &[&str]); 3] = [
        ("a.md", "consumer", "explicit", &["warning"]),
        ("b.md", "internal", "explicit", &["suggestion"]),
        ("c.md", "change-comms", "explicit", &[]),
    ];
    for (suffix, genre, source, levels) in expected {
        let doc = document(&report, suffix);
        assert_eq!(doc["genre"]["id"], genre, "{suffix}: {doc}");
        assert_eq!(doc["genre"]["source"], source, "{suffix}: {doc}");
        assert_eq!(history_levels(doc), levels, "{suffix}: {doc}");
    }
    // The negated pattern frees this file from the block, so no assignment names its genre.
    let freed = document(&report, "d.md");
    assert_ne!(freed["genre"]["source"], "explicit", "{freed}");
}

/// Lint a project with `config` and one document `notes/n.md`, which `CONFIG`-style blocks
/// assign to `internal`. Returns the exit code and the report.
fn lint_note(config: &str, text: &str) -> (Option<i32>, Value) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), config).expect("write config");
    let path = dir.path().join("notes/n.md");
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write document");
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .arg(dir.path())
        .output()
        .expect("run");
    let report = serde_json::from_slice(&out.stdout).expect("a JSON report");
    (out.status.code(), report)
}

const ASSIGNED: &str =
    "schema_version = 3\n\n[[genres]]\nfiles = [\"notes/**\"]\ngenre = \"internal\"\n";

#[test]
fn a_top_level_genre_key_never_overrides_the_assignment() {
    let text = format!("---\ngenre: reference\n---\n{HISTORY}");
    let (_, report) = lint_note(ASSIGNED, &text);
    let doc = document(&report, "n.md");
    assert_eq!(doc["genre"]["id"], "internal", "{doc}");
    assert_eq!(doc["genre"]["source"], "explicit", "{doc}");
    assert_eq!(history_levels(doc), ["suggestion"], "{doc}");
}

#[test]
fn an_unknown_top_level_genre_key_does_not_fail_the_run() {
    let text = format!("---\ngenre: no-such-genre\n---\n{HISTORY}");
    let (code, report) = lint_note(ASSIGNED, &text);
    assert_ne!(code, Some(2), "{report}");
    assert_eq!(document(&report, "n.md")["genre"]["id"], "internal");
}

#[test]
fn a_disabled_front_matter_block_is_ignored() {
    let config = format!("{ASSIGNED}\n[frontmatter]\nenabled = false\n");
    let text = format!("---\nslopvac:\n  v: 3\n  genre: reference\n---\n{HISTORY}");
    let (_, report) = lint_note(&config, &text);
    let doc = document(&report, "n.md");
    assert_eq!(doc["genre"]["id"], "internal", "{doc}");
}

#[test]
fn an_allowed_slopvac_block_genre_wins_over_the_assignment() {
    let text = format!("---\nslopvac:\n  v: 3\n  genre: reference\n---\n{HISTORY}");
    let (_, report) = lint_note(ASSIGNED, &text);
    let doc = document(&report, "n.md");
    assert_eq!(doc["genre"]["id"], "reference", "{doc}");
    assert_eq!(doc["genre"]["source"], "explicit", "{doc}");
    assert_eq!(
        doc["genre"]["reasons"][0]["signal"], "front-matter",
        "{doc}"
    );
    assert_eq!(history_levels(doc), ["warning"], "{doc}");
}
