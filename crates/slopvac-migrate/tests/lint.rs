//! Before and after findings from the engine.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use slopvac_migrate::{
    BACKUP_DIR, EngineLinter, Finding, Linter, Options, Phase, RenameTable, plan,
};
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, RuleSet};

/// One core category `orwell` with a tokens rule `stale-figure` on "leverage" that lists
/// the exception `dead-metaphor`. The 2.x id `orwell.stale-figure` is in the rename table.
fn linter() -> EngineLinter {
    let category = json!({
        "schema_version": 3, "id": "orwell", "title": "t", "description": "d",
        "rules": [{
            "id": "stale-figure", "name": "n", "message": "Found {match}.", "severity": "warning",
            "match": {"kind": "tokens", "tokens": ["leverage"]},
            "exceptions": [{"name": "dead-metaphor"}],
            "citation": {"source": "s"}
        }]
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
fn the_backup_directory_is_not_linted() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "doc.md", "Please leverage it.\n");
    write(
        dir.path(),
        &format!("{BACKUP_DIR}/doc.md"),
        "Please leverage it.\n",
    );

    let before = linter().lint(dir.path(), Phase::Before, &[]).unwrap();

    assert_eq!(counts(&before), [("core:orwell.stale-figure", 1)]);
}
