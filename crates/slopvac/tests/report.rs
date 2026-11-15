//! Installed-binary tests for `lint --format` and `--out` (compat rows
//! `opt:lint:--format`, `opt:lint:--out`, `behavior:report-out-open`).
//!
//! The engine behind the CLI is still STUB(slopvac-8l4.12.12) and finds
//! nothing, so each test checks the shape of an empty-findings report: that the
//! output parses in its format and names the document. Content checks per
//! finding live in `slopvac-report`'s own tests.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// Three words, so the word count is known without running the code.
const TEXT: &str = "one two three\n";

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
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

fn stdout_of(format: &str) -> String {
    let out = slopvac()
        .args(["lint", "--stdin", "--format", format])
        .write_stdin(TEXT)
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("utf-8 output")
}

fn validator() -> jsonschema::Validator {
    let schemas = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3");
    let read = |name: &str| -> Value {
        serde_json::from_str(&fs::read_to_string(schemas.join(name)).expect("schema file"))
            .expect("schema json")
    };
    let registry = jsonschema::Registry::new()
        .add(
            "https://srobroek.github.io/slopvac/schemas/v3/common.schema.json",
            read("common.schema.json"),
        )
        .expect("register common")
        .prepare()
        .expect("prepare registry");
    jsonschema::options()
        .with_registry(&registry)
        .build(&read("lint-report.schema.json"))
        .expect("schema compiles")
}

#[test]
fn text_is_the_default_and_ends_with_a_verdict_line() {
    let default = slopvac()
        .args(["lint", "--stdin"])
        .write_stdin(TEXT)
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(default).unwrap();
    assert_eq!(text, stdout_of("text"));
    assert!(
        text.ends_with(
            "1 documents, 0 findings (0 errors, 0 warnings, 0 suggestions), score 100.0: PASS\n"
        ),
        "{text}"
    );
}

#[test]
fn json_validates_against_the_published_schema() {
    let report: Value = serde_json::from_str(&stdout_of("json")).expect("json");
    let problems: Vec<String> = validator()
        .iter_errors(&report)
        .map(|e| e.to_string())
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(report["schema_version"], 3);
    assert_eq!(report["documents"][0]["path"], "<stdin>");
    assert_eq!(report["documents"][0]["words"], 3);
    assert_eq!(report["summary"]["documents"], 1);
}

#[test]
fn github_prints_no_annotations_without_findings() {
    assert_eq!(stdout_of("github"), "");
}

#[test]
fn sarif_is_version_2_1_0_with_an_empty_result_list() {
    let sarif: Value = serde_json::from_str(&stdout_of("sarif")).expect("json");
    assert_eq!(sarif["version"], "2.1.0");
    assert_eq!(sarif["runs"][0]["tool"]["driver"]["name"], "slopvac");
    assert_eq!(sarif["runs"][0]["results"], Value::Array(vec![]));
}

#[test]
fn rdjsonl_prints_one_object_per_finding_so_none_here() {
    assert_eq!(stdout_of("rdjsonl"), "");
}

#[test]
fn codeclimate_is_an_empty_json_array() {
    let issues: Value = serde_json::from_str(&stdout_of("codeclimate")).expect("json");
    assert_eq!(issues, Value::Array(vec![]));
}

#[test]
fn junit_is_xml_with_one_suite_named_for_the_document() {
    let xml = stdout_of("junit");
    let doc = roxmltree::Document::parse(&xml).expect("xml");
    assert_eq!(doc.root_element().tag_name().name(), "testsuites");
    let suites: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("testsuite"))
        .collect();
    assert_eq!(suites.len(), 1);
    assert_eq!(suites[0].attribute("name"), Some("<stdin>"));
    assert_eq!(suites[0].attribute("failures"), Some("0"));
}

#[test]
fn checkstyle_is_xml_with_one_file_named_for_the_document() {
    let xml = stdout_of("checkstyle");
    let doc = roxmltree::Document::parse(&xml).expect("xml");
    assert_eq!(doc.root_element().tag_name().name(), "checkstyle");
    let files: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("file"))
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].attribute("name"), Some("<stdin>"));
}

#[test]
fn markdown_has_a_heading_and_escapes_the_document_name() {
    let md = stdout_of("markdown");
    assert!(md.starts_with("# slopvac report\n"), "{md}");
    assert!(md.contains("## \\<stdin\\>"), "{md}");
}

#[test]
fn html_is_a_page_that_escapes_the_document_name() {
    let html = stdout_of("html");
    assert!(
        html.to_ascii_lowercase().starts_with("<!doctype html>"),
        "{html}"
    );
    assert!(html.contains("&lt;stdin&gt;"), "{html}");
    assert!(!html.contains("<stdin>"), "{html}");
}

#[test]
fn unknown_format_is_a_usage_error() {
    slopvac()
        .args(["lint", "--stdin", "--format", "pdf"])
        .write_stdin(TEXT)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[usage]: "));
}

#[test]
fn file_documents_keep_the_path_as_given() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("doc.md"), TEXT).expect("write");
    let out = slopvac()
        .current_dir(dir.path())
        .args(["lint", "doc.md", "--format", "json"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let report: Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(report["documents"][0]["path"], "doc.md");
}

#[test]
fn out_writes_the_chosen_format_to_the_file_and_names_it_on_stdout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("report.json");
    slopvac()
        .args(["lint", "--stdin", "--format", "json", "--out"])
        .arg(&target)
        .write_stdin(TEXT)
        .assert()
        .code(0)
        .stdout(format!("report: {}\n", target.display()));
    let report: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(report["schema_version"], 3);
}

#[test]
fn out_without_a_format_writes_html_as_2x_did() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("report.html");
    slopvac()
        .args(["lint", "--stdin", "--out"])
        .arg(&target)
        .write_stdin(TEXT)
        .assert()
        .code(0);
    let page = fs::read_to_string(&target).unwrap();
    assert!(page.to_ascii_lowercase().starts_with("<!doctype html>"));
}

#[test]
fn out_with_an_explicit_text_format_writes_text() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("report.txt");
    slopvac()
        .args(["lint", "--stdin", "--format", "text", "--out"])
        .arg(&target)
        .write_stdin(TEXT)
        .assert()
        .code(0);
    assert!(
        fs::read_to_string(&target)
            .unwrap()
            .contains("score 100.0: PASS")
    );
}

#[test]
fn out_creates_missing_parent_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("a/b/report.json");
    slopvac()
        .args(["lint", "--stdin", "--format", "json", "--out"])
        .arg(&target)
        .write_stdin(TEXT)
        .assert()
        .code(0);
    assert!(target.is_file());
}

#[test]
fn an_unwritable_out_path_is_an_incomplete_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A file stands where the parent directory should be.
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "").unwrap();
    slopvac()
        .args(["lint", "--stdin", "--out"])
        .arg(blocker.join("report.html"))
        .write_stdin(TEXT)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[output-failed]: "));
}

#[test]
fn invalid_utf8_stdin_writes_no_report() {
    slopvac()
        .args(["lint", "--stdin", "--format", "json"])
        .write_stdin(&b"\xff"[..])
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty());
}

#[test]
fn skipped_inputs_become_notices_with_codes_and_no_absolute_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("good.md"), TEXT).unwrap();
    fs::write(dir.path().join("bad.md"), b"\xff\xfe").unwrap();
    let out = slopvac()
        .current_dir(dir.path())
        .args([
            "lint",
            "good.md",
            "bad.md",
            "missing.md",
            "--format",
            "json",
        ])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let report: Value = serde_json::from_slice(&out).expect("json");
    assert!(validator().is_valid(&report), "{report}");
    assert_eq!(report["documents"].as_array().unwrap().len(), 1);
    let notices: Vec<(&str, &str, &str)> = report["notices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            (
                n["code"].as_str().unwrap(),
                n["level"].as_str().unwrap(),
                n["path"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        notices,
        [
            ("input-not-utf8", "warning", "bad.md"),
            ("input-unreadable", "warning", "missing.md"),
        ]
    );
    assert!(
        !report
            .to_string()
            .contains(&dir.path().display().to_string())
    );
}

#[cfg(unix)]
#[test]
fn a_failed_out_write_keeps_the_previous_report() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("report.json");
    fs::write(&target, "previous").unwrap();
    // The file stays writable but the directory refuses new entries, so only a
    // truncate-in-place write could still succeed or damage it.
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
    if fs::File::create(dir.path().join("probe")).is_ok() {
        return; // running with privileges that ignore permissions
    }
    slopvac()
        .args(["lint", "--stdin", "--format", "json", "--out"])
        .arg(&target)
        .write_stdin(TEXT)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[output-failed]: "));
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "previous");
}

#[test]
fn sarif_says_the_run_failed_when_an_input_was_skipped() {
    let out = slopvac()
        .args(["lint", "missing.md", "--format", "sarif"])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let sarif: Value = serde_json::from_slice(&out).expect("sarif json");
    let invocation = &sarif["runs"][0]["invocations"][0];
    assert_eq!(invocation["executionSuccessful"], false);
    assert_eq!(
        invocation["toolExecutionNotifications"][0]["descriptor"]["id"],
        "input-unreadable"
    );
}
