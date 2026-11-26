//! The run summary keeps two plugins' categories apart when they share a bare category id.
use std::error::Error;
use std::path::PathBuf;

use serde_json::json;
use slopvac_cli::summary::summarize;
use slopvac_engine::{Engine, Input, Settings};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString, PackId};
use slopvac_rules::{CompileOptions, RuleSet};

/// A rule set of `plugin` with the category `style` and one rule that flags `token`.
fn plugin_set(plugin: &str, token: &str) -> Result<RuleSet, Box<dyn Error>> {
    let category = json!({
        "schema_version": 3, "id": "style", "title": "t", "description": "d",
        "rules": [{
            "id": "hit", "name": "n", "message": "Found {match}.", "severity": "warning",
            "match": {"kind": "tokens", "tokens": [token]},
            "citation": {"source": "s"}
        }],
    });
    Ok(RuleSet::from_sources(
        &PackId::new(plugin)?,
        vec![(PathBuf::from("style.json"), category.to_string())],
        &CompileOptions::default(),
    )?)
}

#[test]
fn a_shared_bare_category_id_stays_two_rows_in_the_summary() -> Result<(), Box<dyn Error>> {
    let settings = Settings::new(
        NonEmptyString::new("normal")?,
        GenreResult {
            id: GenreId::new("reference")?,
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    let engine = Engine::new(
        vec![
            plugin_set("alpha", "leverage")?,
            plugin_set("beta", "synergy")?,
        ],
        settings,
    )?;
    // alpha flags one word, beta flags two.
    let text = "leverage synergy synergy word\n";
    let document = engine.lint(&Input::markdown("doc.md", text))?;

    let summary = summarize(&[document]);

    let rows: Vec<(&str, usize)> = summary
        .categories
        .iter()
        .map(|c| (c.category.as_str(), c.findings))
        .collect();
    assert_eq!(rows, [("alpha:style", 1), ("beta:style", 2)]);
    Ok(())
}
