//! Before and after findings from the engine.
#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use slopvac_migrate::{
    BACKUP_DIR, EngineLinter, Error, Finding, Linter, Options, Phase, RenameTable, plan,
};
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
    EngineLinter::new(vec![set]).unwrap()
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
fn before_and_after_counts_come_from_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "doc.md",
        "<!-- slopvac-allow: rule=orwell.stale-figure reason=dead-metaphor -->\nPlease leverage it.\n\nWe leverage more.\n",
    );
    // The document has no steering block, so the managed block is never written.
    let p = plan(dir.path(), &RenameTable::bundled(), Options::default(), "").unwrap();
    let linter = linter();

    let before = linter.lint(dir.path(), Phase::Before, &[]).unwrap();
    let after = linter
        .lint(dir.path(), Phase::After, &p.overlay(dir.path()))
        .unwrap();

    // Before, the 2.x id has no pack: the directive is reported and suppresses nothing.
    assert_eq!(
        counts(&before),
        [
            ("core:meta.invalid-suppression", 1),
            ("core:orwell.stale-figure", 2)
        ]
    );
    // After, the qualified id suppresses the paragraph below it and no other.
    assert_eq!(counts(&after), [("core:orwell.stale-figure", 1)]);
}

#[test]
fn backup_directories_at_any_depth_are_not_linted() {
    for backup in [BACKUP_DIR.to_owned(), format!("nested/{BACKUP_DIR}")] {
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
        &format!("{BACKUP_DIR}/doc.md"),
        "Please leverage it.\n",
    );
    let backup = dir.path().join(BACKUP_DIR);
    fs::set_permissions(&backup, fs::Permissions::from_mode(0o000)).unwrap();

    let before = linter().lint(dir.path(), Phase::Before, &[]);

    fs::set_permissions(&backup, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(counts(&before.unwrap()), [("core:orwell.stale-figure", 1)]);
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
