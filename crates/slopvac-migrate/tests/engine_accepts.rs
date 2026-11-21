//! The 3.0 engine against what `migrate` writes for directives.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

use std::path::PathBuf;

use serde_json::{Value, json};
use slopvac_engine::{Engine, Input, Settings};
use slopvac_migrate::{RenameTable, rewrite_directives};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString, PackId};
use slopvac_rules::{CompileOptions, RuleSet};

fn rule(id: &str, token: &str) -> Value {
    json!({
        "id": id, "name": "n", "message": "Found {match}.", "severity": "warning",
        "match": {"kind": "tokens", "tokens": [token]},
        "citation": {"source": "s"},
        "exceptions": [{"name": "ok"}]
    })
}

fn engine() -> Engine {
    let category = json!({
        "schema_version": 3, "id": "cat", "title": "t", "description": "d",
        "rules": [rule("one", "alpha"), rule("two", "beta")]
    });
    let set = RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        vec![(PathBuf::from("cat.json"), category.to_string())],
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let settings = Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new("reference").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    Engine::new(vec![set], settings).unwrap()
}

/// `cat.one` and `cat.two` stand in for two 2.x ids; the table maps both into core.
fn table() -> RenameTable {
    RenameTable::parse(
        "[rules]\n\"cat.one\" = \"core:cat.one\"\n\"cat.two\" = \"core:cat.two\"\n[categories]\n[removed]\nrules = [\"cat.gone\"]\n",
    )
    .unwrap()
}

fn rules_found(source: &str) -> Vec<String> {
    let doc = engine().lint(&Input::markdown("a.md", source)).unwrap();
    doc.findings.iter().map(|f| f.rule_id.to_string()).collect()
}

#[test]
fn a_renamed_single_id_directive_suppresses_in_3_0() {
    let old = "<!-- slopvac-allow: rule=cat.one reason=ok -->\nalpha\n";
    let (migrated, _) = rewrite_directives(old, &table());
    assert_eq!(rules_found(&migrated), Vec::<String>::new(), "{migrated}");
}

#[test]
fn a_comma_list_is_left_as_written_and_the_engine_still_rejects_it() {
    let old = "<!-- slopvac-allow: rule=cat.one,cat.two reason=ok -->\nalpha beta\n";
    let (migrated, _) = rewrite_directives(old, &table());
    assert_eq!(migrated, old);
    // The invalid directive suppresses nothing: both hits stay next to the error.
    assert_eq!(
        rules_found(&migrated),
        [
            "core:meta.invalid-suppression",
            "core:cat.one",
            "core:cat.two"
        ],
    );
}
