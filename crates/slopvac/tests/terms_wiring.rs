//! Terms in `slopvac lint` and `slopvac lsp`: project terms files, `terms.allow`, blocklists
//! and the term-casing rule (bead 8.10 acceptance: a project term spelled wrongly is flagged,
//! `terms.allow` suppresses it, the language server reports the same).
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

mod support;

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    // Tests stay off the network: the update notice may refresh its cache after lint, doctor and prime.
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

const TERMS_FILE: &str =
    "schema_version = 3\n\n[[terms]]\nterm = \"Zorblax\"\nkind = \"product\"\n";
const BLOCKLIST: &str = "schema_version = 3\n\n[[entries]]\nword = \"synergize\"\npos = \"verb\"\nreason = \"Business jargon.\"\n";
const MISCASED: &str = "# Title\n\nThe team deploys zorblax on Friday.\n";

/// A project with the given config, extra files and one document.
fn project(config: &str, extra: &[(&str, &str)], doc: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), config).expect("config");
    for (name, text) in extra {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, text).expect("extra file");
    }
    let path = dir.path().join("doc.md");
    fs::write(&path, doc).expect("doc");
    (dir, path)
}

fn rule_ids(path: &Path) -> Vec<String> {
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

#[test]
fn a_project_terms_file_enforces_its_casing() {
    let config = "schema_version = 3\n\n[terms]\nfiles = [\"terms.toml\"]\n";
    let (_dir, path) = project(config, &[("terms.toml", TERMS_FILE)], MISCASED);
    assert_eq!(rule_ids(&path), ["terms:terms.casing"]);
    slopvac()
        .arg("lint")
        .arg(&path)
        .assert()
        .stdout(predicate::str::contains("Zorblax"));
}

#[test]
fn without_the_terms_file_the_term_is_not_known() {
    let (_dir, path) = project("schema_version = 3\n", &[], MISCASED);
    assert_eq!(rule_ids(&path), Vec::<String>::new());
}

#[test]
fn terms_file_paths_are_relative_to_the_config_that_names_them() {
    let config = "schema_version = 3\n\n[terms]\nfiles = [\"data/terms.toml\"]\n";
    let (_dir, path) = project(config, &[("data/terms.toml", TERMS_FILE)], MISCASED);
    assert_eq!(rule_ids(&path), ["terms:terms.casing"]);
}

#[test]
fn terms_allow_suppresses_the_finding() {
    let config = "schema_version = 3\n\n[terms]\nfiles = [\"terms.toml\"]\nallow = [\"zorblax\"]\n";
    let (_dir, path) = project(config, &[("terms.toml", TERMS_FILE)], MISCASED);
    assert_eq!(rule_ids(&path), Vec::<String>::new());
}

#[test]
fn the_built_in_terms_still_apply_next_to_project_terms() {
    let config = "schema_version = 3\n\n[terms]\nfiles = [\"terms.toml\"]\n";
    let doc = "# Title\n\nThe team uses Github for the code.\n";
    let (_dir, path) = project(config, &[("terms.toml", TERMS_FILE)], doc);
    assert_eq!(rule_ids(&path), ["terms:terms.casing"]);
}

#[test]
fn a_blocklist_entry_is_flagged_and_without_one_the_run_says_so() {
    let doc = "# Title\n\nManagers synergize the teams on Friday.\n";
    let config = "schema_version = 3\n\n[terms]\nblocklists = [\"words.toml\"]\n";
    let (_dir, path) = project(config, &[("words.toml", BLOCKLIST)], doc);
    let ids = rule_ids(&path);
    assert!(ids.iter().any(|id| id.contains("vocabulary")), "{ids:?}");
    let (_dir2, bare) = project("schema_version = 3\n", &[], doc);
    assert!(
        !rule_ids(&bare).iter().any(|id| id.contains("vocabulary")),
        "no blocklist, no finding"
    );
}

#[test]
fn an_unreadable_terms_file_exits_2_without_its_directories() {
    let config = "schema_version = 3\n\n[terms]\nfiles = [\"missing.toml\"]\n";
    let (dir, path) = project(config, &[], MISCASED);
    let sentinel = dir
        .path()
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned();
    let out = slopvac().arg("lint").arg(&path).output().expect("run");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.starts_with("error[terms-invalid]"), "{stderr}");
    assert!(!stderr.contains(&sentinel), "{stderr}");
}

/// A refused terms file or blocklist reports the entry number and the kind of failure, and
/// never the words in the file: the message reaches stderr, reports and the language server.
#[rstest::rstest]
#[case::blocklist_unknown_pos(
    "blocklists",
    "words.toml",
    "schema_version = 3\n\n[[entries]]\nword = \"SENTINELWORD\"\npos = \"wizard\"\nreason = \"r\"\n",
    "entry 1 has an unrecognized part of speech"
)]
#[case::blocklist_no_reason(
    "blocklists",
    "words.toml",
    "schema_version = 3\n\n[[entries]]\nword = \"ok\"\npos = \"verb\"\nreason = \"r\"\n\n[[entries]]\nword = \"SENTINELWORD\"\npos = \"verb\"\nreason = \" \"\n",
    "entry 2 has no reason"
)]
#[case::blocklist_duplicate(
    "blocklists",
    "words.toml",
    "schema_version = 3\n\n[[entries]]\nword = \"SENTINELWORD\"\npos = \"verb\"\nreason = \"r\"\n\n[[entries]]\nword = \"SENTINELWORD\"\npos = \"verb\"\nreason = \"r\"\n",
    "entry 2 is a duplicate"
)]
#[case::blocklist_unknown_key(
    "blocklists",
    "words.toml",
    "schema_version = 3\nentries = []\nSENTINELKEY = 1\n",
    "has an unknown key"
)]
#[case::blocklist_syntax(
    "blocklists",
    "words.toml",
    "schema_version = 3\n[[entries]]\nword = SENTINELSYNTAX\n",
    "is malformed"
)]
#[case::terms_duplicate(
    "files",
    "terms.toml",
    "schema_version = 3\n\n[[terms]]\nterm = \"SENTINELTERM\"\n\n[[terms]]\nterm = \"SENTINELTERM\"\n",
    "entry 2 is a duplicate"
)]
#[case::terms_conflicting_casing(
    "files",
    "terms.toml",
    "schema_version = 3\n\n[[terms]]\nterm = \"SENTINELTERM\"\nignore_case = true\ncase_sensitive = true\n",
    "entry 1 sets both casing modes"
)]
fn a_refused_terms_input_is_described_without_its_words(
    #[case] key: &str,
    #[case] name: &str,
    #[case] text: &str,
    #[case] expected: &str,
) {
    let config = format!("schema_version = 3\n\n[terms]\n{key} = [\"{name}\"]\n");
    let (_dir, path) = project(&config, &[(name, text)], MISCASED);
    let out = slopvac().arg("lint").arg(&path).output().expect("run");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.starts_with("error[terms-invalid]"), "{stderr}");
    assert!(stderr.contains(expected), "{stderr}");
    assert!(!stderr.contains("SENTINEL"), "{stderr}");
}

#[test]
fn a_terms_file_outside_the_project_shows_as_external_even_when_missing() {
    let config =
        "schema_version = 3\n\n[terms]\nfiles = [\"../SENTINELDIR/nested/../terms.toml\"]\n";
    let (_dir, path) = project(config, &[], MISCASED);
    let out = slopvac().arg("lint").arg(&path).output().expect("run");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("<external>/terms.toml"), "{stderr}");
    assert!(!stderr.contains("SENTINELDIR"), "{stderr}");
    assert!(!stderr.contains(".."), "{stderr}");
}

#[test]
fn projects_with_the_same_settings_and_different_terms_files_stay_apart() {
    // Both projects have the same config text, so their settings are equal; only the terms
    // files differ. One run must lint each document against its own project's terms.
    let config = "schema_version = 3\n\n[terms]\nfiles = [\"terms.toml\"]\n";
    let doc = "# Title\n\nThe team deploys zorblax and quuxify on Friday.\n";
    let dir = tempfile::tempdir().expect("tempdir");
    for (project, term) in [("a", "Zorblax"), ("b", "Quuxify")] {
        let root = dir.path().join(project);
        fs::create_dir_all(&root).expect("project");
        fs::write(root.join("slopvac.toml"), config).expect("config");
        fs::write(
            root.join("terms.toml"),
            format!("schema_version = 3\n\n[[terms]]\nterm = \"{term}\"\n"),
        )
        .expect("terms");
        fs::write(root.join("doc.md"), doc).expect("doc");
    }
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "--format", "json", "a/doc.md", "b/doc.md"])
        .output()
        .expect("run");
    let report: Value = serde_json::from_slice(&out.stdout).expect("json report");
    let matched = |path: &str| -> Vec<String> {
        report["documents"]
            .as_array()
            .expect("documents")
            .iter()
            .find(|d| d["path"] == path)
            .expect("document")["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|f| f["matched_text"].as_str().expect("matched").to_owned())
            .collect()
    };
    assert_eq!(matched("a/doc.md"), ["zorblax"]);
    assert_eq!(matched("b/doc.md"), ["quuxify"]);
}

#[test]
fn a_category_qualified_with_the_wrong_pack_is_a_config_error() {
    // `orwell` is a core category; the `terms` pack has no such category.
    let config = "schema_version = 3\n\n[categories]\n\"terms:orwell\" = \"off\"\n";
    let (_dir, path) = project(config, &[], MISCASED);
    slopvac()
        .arg("lint")
        .arg(&path)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[config-unknown-category]"));
}

#[test]
fn disabling_a_category_with_the_wrong_pack_does_not_disable_the_core_category() {
    let (_dir, path) = project("schema_version = 3\n", &[], MISCASED);
    slopvac()
        .args(["lint", "--disable", "terms:orwell"])
        .arg(&path)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown category or rule"));
}

#[test]
fn a_category_qualified_with_its_own_pack_still_resolves() {
    let config = "schema_version = 3\n\n[categories]\n\"core:orwell\" = \"off\"\n";
    let (_dir, path) = project(config, &[], MISCASED);
    slopvac().arg("lint").arg(&path).assert().code(0);
}

#[test]
fn the_term_casing_rule_can_be_turned_off_by_its_id() {
    let config = "schema_version = 3\n\n[terms]\nfiles = [\"terms.toml\"]\n\n[rules]\n\"terms:terms.casing\" = \"off\"\n";
    let (_dir, path) = project(config, &[("terms.toml", TERMS_FILE)], MISCASED);
    assert_eq!(rule_ids(&path), Vec::<String>::new());
}

fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

/// The messages a `slopvac lsp` session writes for `text` opened in `dir`.
fn lsp_output(dir: &Path, text: &str) -> String {
    // The document sits in the project, so the project config applies to it.
    let uri = format!("file://{}/doc.md", dir.display());
    let open = serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {"textDocument": {
            "uri": uri, "languageId": "markdown", "version": 1, "text": text}}
    });
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}"#.to_owned(),
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#.to_owned(),
        open.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#.to_owned(),
        r#"{"jsonrpc":"2.0","method":"exit"}"#.to_owned(),
    ]
    .iter()
    .map(|m| frame(m))
    .collect::<String>();
    let out = slopvac()
        .current_dir(dir)
        .arg("lsp")
        .write_stdin(input)
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn the_language_server_reports_a_project_term_and_honours_terms_allow() {
    let config = "schema_version = 3\n\n[terms]\nfiles = [\"terms.toml\"]\n";
    let (dir, _) = project(config, &[("terms.toml", TERMS_FILE)], MISCASED);
    let reported = lsp_output(dir.path(), MISCASED);
    assert!(reported.contains("terms:terms.casing"), "{reported}");

    let allowed =
        "schema_version = 3\n\n[terms]\nfiles = [\"terms.toml\"]\nallow = [\"zorblax\"]\n";
    let (dir2, _) = project(allowed, &[("terms.toml", TERMS_FILE)], MISCASED);
    let quiet = lsp_output(dir2.path(), MISCASED);
    assert!(!quiet.contains("terms:terms.casing"), "{quiet}");
}
