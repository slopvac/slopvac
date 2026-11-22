//! Before and after findings from the engine.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use slopvac_migrate::{EngineLinter, Error, Finding, Linter, Options, Phase, RenameTable, plan};
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, RuleSet};

/// A tokens rule `stale-figure` on "leverage" that lists the exception `dead-metaphor`.
/// The 2.x id `orwell.stale-figure` is in the rename table.
fn stale_figure() -> Value {
    json!({
        "id": "stale-figure", "name": "n", "message": "Found {match}.", "severity": "warning",
        "match": {"kind": "tokens", "tokens": ["leverage"]},
        "exceptions": [{"name": "dead-metaphor"}],
        "citation": {"source": "s"}
    })
}

/// An engine over one core category `orwell` holding [`stale_figure`].
fn linter() -> EngineLinter {
    linter_with(&[stale_figure()])
}

fn linter_with(rules: &[Value]) -> EngineLinter {
    let category = json!({
        "schema_version": 3, "id": "orwell", "title": "t", "description": "d",
        "rules": rules
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

fn counts(findings: &[Finding]) -> Vec<(&str, usize)> {
    findings
        .iter()
        .map(|f| (f.rule.as_str(), f.count))
        .collect()
}

#[test]
fn a_2x_bare_id_suppression_holds_on_the_before_side() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "doc.md",
        "<!-- slopvac-allow: rule=orwell.stale-figure reason=dead-metaphor -->\nPlease leverage it.\n\nWe leverage more.\n",
    );
    // The document has no steering block, so the managed block is never written.
    let p = plan(dir.path(), &RenameTable::bundled(), Options::default(), "").unwrap();
    let linter = linter();

    let before = linter
        .lint(dir.path(), Phase::Before, &p.before_overlay(dir.path()))
        .unwrap();
    let after = linter
        .lint(dir.path(), Phase::After, &p.overlay(dir.path()))
        .unwrap();

    // 2.x accepts the bare id, so on both sides it suppresses the paragraph below it and
    // no other, and no side reports it as an invalid suppression.
    assert_eq!(counts(&before), [("core:orwell.stale-figure", 1)]);
    assert_eq!(counts(&after), [("core:orwell.stale-figure", 1)]);
}

#[test]
fn a_2x_suppression_of_a_removed_rule_is_not_a_finding_on_the_before_side() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "doc.md",
        "<!-- slopvac-allow: rule=ste-words.inconsistent-term-for-same-thing reason=r -->\nPlease leverage it.\n",
    );
    let p = plan(dir.path(), &RenameTable::bundled(), Options::default(), "").unwrap();

    let before = linter()
        .lint(dir.path(), Phase::Before, &p.before_overlay(dir.path()))
        .unwrap();

    assert_eq!(counts(&before), [("core:orwell.stale-figure", 1)]);
}

#[test]
fn backup_directories_at_any_depth_are_not_linted() {
    for backup in [
        ".slopvac/migrate-backup/20261120T064000Z",
        "nested/.slopvac/migrate-backup/20261120T064000Z",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "doc.md", "Please leverage it.\n");
        write(dir.path(), "nested/doc.md", "Please leverage it.\n");
        write(
            dir.path(),
            &format!("{backup}/doc.md"),
            "Please leverage it.\n",
        );

        let before = linter().lint(dir.path(), Phase::Before, &[]).unwrap();

        assert_eq!(
            counts(&before),
            [("core:orwell.stale-figure", 2)],
            "{backup}"
        );
    }
}

#[cfg(unix)]
#[test]
fn an_unreadable_backup_directory_is_never_read() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "doc.md", "Please leverage it.\n");
    write(
        dir.path(),
        ".slopvac/migrate-backup/20261120T064000Z/doc.md",
        "Please leverage it.\n",
    );
    let backup = dir.path().join(".slopvac/migrate-backup");
    fs::set_permissions(&backup, fs::Permissions::from_mode(0o000)).unwrap();

    let before = linter().lint(dir.path(), Phase::Before, &[]);

    fs::set_permissions(&backup, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(counts(&before.unwrap()), [("core:orwell.stale-figure", 1)]);
}

#[test]
fn documents_beside_the_backup_directory_are_linted() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".slopvac/doc.md", "Please leverage it.\n");
    write(dir.path(), ".slopvac/other/doc.md", "Please leverage it.\n");

    let before = linter().lint(dir.path(), Phase::Before, &[]).unwrap();

    assert_eq!(counts(&before), [("core:orwell.stale-figure", 2)]);
}

#[test]
fn mdx_prose_is_linted_and_jsx_is_not() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "doc.mdx",
        "import { Note } from './note'\n\nPlease leverage it.\n\n<Note title=\"leverage\" />\n\n{props.leverage}\n",
    );

    let before = linter().lint(dir.path(), Phase::Before, &[]).unwrap();

    // The paragraph counts. The import, the JSX attribute and the expression, which is
    // not a string literal, are code.
    assert_eq!(counts(&before), [("core:orwell.stale-figure", 1)]);
}

#[test]
fn a_warning_that_part_of_the_check_did_not_run_fails_the_lint() {
    // The engine runs only the sentence style of capitalization, so a title-style rule
    // leaves every document with a warning-level unchecked notice.
    let title_case = json!({
        "id": "title-heading", "name": "n", "message": "m", "severity": "warning",
        "match": {"kind": "capitalization", "style": "title"},
        "citation": {"source": "s"}
    });
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "doc.md", "Please leverage it.\n");

    let result = linter_with(&[stale_figure(), title_case]).lint(dir.path(), Phase::Before, &[]);

    let Err(Error::Lint(message)) = result else {
        panic!("expected a lint failure, got {result:?}");
    };
    assert!(message.contains("rule-style-not-implemented"), "{message}");
    assert_eq!(Error::Lint(message).code(), "migrate-lint-failed");
}
