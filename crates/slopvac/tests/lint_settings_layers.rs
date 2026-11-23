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

/// Lint `dir`'s `path` and return the document object of the JSON report.
fn document_at(dir: &std::path::Path, path: &str) -> Value {
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .arg(dir.join(path))
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("json report");
    report["documents"][0].clone()
}

#[test]
fn a_detected_genre_feeds_its_profile_into_the_settings() {
    // `reference/` is a path signal for the `reference` genre, whose file sets `profile:
    // strict`. No config or front matter names a genre, so detection chooses it.
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), "schema_version = 3\n").expect("config");
    fs::create_dir(dir.path().join("reference")).expect("dir");
    fs::write(dir.path().join("reference/e.md"), BODY).expect("doc");
    let doc = document_at(dir.path(), "reference/e.md");
    assert_eq!(doc["genre"]["source"], "detected", "{doc}");
    assert_eq!(doc["genre"]["id"], "reference", "{doc}");
    assert_eq!(doc["profile"], "strict", "{doc}");
}

#[test]
fn an_explicit_genre_outranks_the_detected_one_and_keeps_its_source() {
    // The path says `reference`; the front matter says `informal` (profile: relaxed).
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), "schema_version = 3\n").expect("config");
    fs::create_dir(dir.path().join("reference")).expect("dir");
    let text = format!("---\nslopvac:\n  v: 3\n  genre: informal\n---\n{BODY}");
    fs::write(dir.path().join("reference/e.md"), text).expect("doc");
    let doc = document_at(dir.path(), "reference/e.md");
    assert_eq!(doc["genre"]["source"], "explicit", "{doc}");
    assert_eq!(doc["genre"]["id"], "informal", "{doc}");
    assert_eq!(doc["profile"], "relaxed", "{doc}");
}

/// A run over `doc.md` with `text`: the exit code, the standard error and the report.
fn run_doc(text: &str) -> (Option<i32>, String, Value) {
    let (_dir, path) = project("schema_version = 3\n", text);
    let out = slopvac()
        .args(["lint", "--format", "json"])
        .arg(&path)
        .output()
        .expect("run");
    let report = serde_json::from_slice(&out.stdout).expect("json report");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        report,
    )
}

/// An inline directive is checked like the front-matter block: a field the project does not
/// allow, or a key that is no setting, is an error at its key, and the run is incomplete
/// (docs/contract/config.md, section 1.1, and the exit codes of `docs/engineering.md`).
/// `<!-- slopvac: ` is 14 bytes, so the key starts at column 15 of line 1.
#[rstest::rstest]
#[case::a_field_the_project_does_not_allow("profile=strict", "config-frontmatter-field-denied")]
#[case::a_key_that_is_no_setting("network.offline=true", "config-unknown-key")]
fn a_bad_inline_directive_is_an_error_and_the_run_is_incomplete(
    #[case] payload: &str,
    #[case] code: &str,
) {
    let (status, stderr, _) = run_doc(&format!("<!-- slopvac: {payload} -->\n{BODY}"));
    assert_eq!(status, Some(2), "{stderr}");
    assert!(stderr.contains(code), "{stderr}");
    assert!(stderr.contains("doc.md:1:15:"), "{stderr}");
}

#[test]
fn an_inline_genre_is_the_genre_of_the_document_and_of_its_settings() {
    // `informal` has `profile: relaxed` and the directive layer sits above detection.
    let (status, stderr, report) = run_doc(&format!("<!-- slopvac: genre=informal -->\n{BODY}"));
    assert_eq!(status, Some(0), "{stderr}");
    let doc = &report["documents"][0];
    assert_eq!(doc["genre"]["id"], "informal", "{doc}");
    assert_eq!(doc["genre"]["source"], "explicit", "{doc}");
    assert_eq!(doc["profile"], "relaxed", "{doc}");
}

/// The `--explain-config` text of `doc.md` with `text`, in a project with `CONFIG`.
fn explain_doc(text: &str) -> String {
    let (_dir, path) = project(CONFIG, text);
    explain(&path)
}

/// The explanation uses the inputs of the lint (docs/contract/config.md, section 1.4.1): a
/// value from the block or a directive names the document, the line and the column of its key.
#[test]
fn the_explanation_shows_a_front_matter_value_at_its_line_and_column() {
    // `    allow:` sits on line 5, and its key starts at column 5.
    let text = explain_doc("---\nslopvac:\n  v: 3\n  terms:\n    allow: [zorblax]\n---\n# T\n");
    assert!(
        text.contains("terms.allow = [\"zorblax\"]  [front-matter "),
        "{text}"
    );
    assert!(text.contains("doc.md:5:5]"), "{text}");
}

#[test]
fn the_explanation_shows_a_directive_value_at_its_line_and_column() {
    // The directive is on line 3 and `<!-- slopvac: ` is 14 bytes, so the key is at column 15.
    let text = explain_doc("# T\n\n<!-- slopvac: locale=en-GB -->\n");
    assert!(text.contains("locale = \"en-GB\"  [directive "), "{text}");
    assert!(text.contains("doc.md:3:15]"), "{text}");
}

#[test]
fn the_explanation_follows_the_detected_genre() {
    // `reference/` is a path signal for `reference`, whose genre file sets `profile: strict`.
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), "schema_version = 3\n").expect("config");
    fs::create_dir(dir.path().join("reference")).expect("dir");
    fs::write(dir.path().join("reference/e.md"), BODY).expect("doc");
    let text = explain(&dir.path().join("reference/e.md"));
    assert!(
        text.contains("profile.name = \"strict\"  [genre plugin:core/genres/reference.yaml:4:1]"),
        "{text}"
    );
}
