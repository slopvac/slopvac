//! Caps, the run budget and hostile text, through the real engine linter.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use slopvac_migrate::{EngineLinter, Limits, Linter, Phase};
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, RuleSet};

fn linter() -> EngineLinter {
    let category = json!({
        "schema_version": 3, "id": "orwell", "title": "t", "description": "d",
        "rules": [{
            "id": "stale-figure", "name": "n", "message": "Found {match}.", "severity": "warning",
            "match": {"kind": "tokens", "tokens": ["leverage"]},
            "citation": {"source": "s"}
        }]
    });
    let set = RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        [(PathBuf::from("orwell.json"), category.to_string())],
        &CompileOptions::default(),
    )
    .unwrap();
    EngineLinter::new(vec![set], Box::new(support::NoConfig))
}

fn write(root: &Path, name: &str, text: &str) {
    let p = root.join(name);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn sparse(path: &Path, len: u64) {
    fs::File::create(path).unwrap().set_len(len).unwrap();
}

#[test]
fn a_document_over_the_cap_fails_the_before_lint_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    sparse(&dir.path().join("big.md"), 64 * 1024 * 1024 + 1);
    let e = linter().lint(dir.path(), Phase::Before, &[]).unwrap_err();
    assert_eq!(e.code(), "document-too-large");
    assert!(e.to_string().contains("big.md: "), "{e}");
}

#[test]
fn a_rewritten_document_over_the_cap_fails_the_after_lint_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "doc.md", "small\n");
    let overlay = vec![(dir.path().join("doc.md"), "x".repeat(64 * 1024 * 1024 + 1))];
    let e = linter()
        .lint(dir.path(), Phase::After, &overlay)
        .unwrap_err();
    assert_eq!(e.code(), "document-too-large");
    assert!(e.to_string().contains("doc.md: "), "{e}");
}

#[test]
fn rewritten_documents_count_against_the_byte_budget() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "doc.md", "a\n");
    let overlay = vec![(dir.path().join("doc.md"), "0123456789\n".to_owned())];
    let e = linter()
        .with_limits(Limits::new(10, 5))
        .lint(dir.path(), Phase::After, &overlay)
        .unwrap_err();
    assert_eq!(e.code(), "migrate-run-limit");
    assert!(e.to_string().contains("doc.md: "), "{e}");
}

#[test]
fn discovery_counts_against_the_file_limit() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..5 {
        write(dir.path(), &format!("f{i}.rs"), "");
    }
    let e = linter()
        .with_limits(Limits::new(3, u64::MAX))
        .lint(dir.path(), Phase::Before, &[])
        .unwrap_err();
    assert_eq!(e.code(), "migrate-run-limit");
}
