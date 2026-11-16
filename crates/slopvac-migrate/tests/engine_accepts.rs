//! The 3.0 engine accepts what `migrate` writes for directives.

#![allow(missing_docs)]

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

#[test]
fn a_directive_with_two_surviving_ids_migrates_to_directives_the_engine_accepts() {
    // `cat.one` and `cat.two` stand in for two 2.x ids; the table maps both into core.
    let table = RenameTable::parse(
        "[rules]\n\"cat.one\" = \"core:cat.one\"\n\"cat.two\" = \"core:cat.two\"\n[categories]\n[removed]\nrules = [\"cat.gone\"]\n",
    )
    .unwrap();
    let old = "<!-- slopvac-allow: rule=cat.one,cat.gone,cat.two reason=ok -->\nalpha beta\n";
    let (migrated, _) = rewrite_directives(old, &table);
    let doc = engine().lint(&Input::markdown("a.md", &migrated)).unwrap();
    assert_eq!(
        doc.findings
            .iter()
            .map(|f| f.rule_id.as_str())
            .collect::<Vec<_>>(),
        Vec::<&str>::new(),
        "{migrated}"
    );
}
