//! Genre tolerances reach the findings: `docs-discipline.history-internal-reference` is off in
//! change-comms, a suggestion in internal and a warning in consumer and reference. Explicit
//! configuration sits above a genre default (`docs/contract/config.md`, section 1.1).
#![cfg(test)]

#[expect(
    dead_code,
    reason = "each test binary uses the part of the shared helpers it needs"
)]
mod common;

use common::{category, id, ruleset, tokens};
use serde_json::json;
use slopvac_engine::{CategoryDials, Engine, Input, Level, Settings};
use slopvac_model::{
    CategoryId, Document, GenreId, GenreResult, GenreSource, NonEmptyString, Severity,
};

fn settings_for(genre: &str) -> Settings {
    Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new(genre).unwrap(),
            // STUB(slopvac-8l4.5.17): the run carries one genre. The config layer that resolves a genre
            // per document and builds `Settings` from it does not exist yet.
            source: GenreSource::Explicit,
            reasons: Vec::new(),
        },
    )
}

/// The rule ships as an error so that any genre level shows up as a change.
fn lint(settings: Settings) -> Document {
    let rules = [tokens(
        "history-internal-reference",
        "refactored",
        &json!({"severity": "error"}),
    )];
    let set = ruleset(vec![category("docs-discipline", &rules)]);
    Engine::new(vec![set], settings)
        .unwrap()
        .lint(&Input::markdown("a.md", "We refactored the parser.\n"))
        .unwrap()
}

fn severities(doc: &Document) -> Vec<Severity> {
    doc.findings.iter().map(|f| f.severity).collect()
}

#[test]
fn the_history_rule_reports_at_the_level_of_the_genre() {
    for (genre, want) in [
        ("change-comms", vec![]),
        ("internal", vec![Severity::Suggestion]),
        ("consumer", vec![Severity::Warning]),
        ("reference", vec![Severity::Warning]),
    ] {
        assert_eq!(severities(&lint(settings_for(genre))), want, "{genre}");
    }
}

#[test]
fn a_genre_without_a_tolerance_keeps_the_shipped_severity() {
    assert_eq!(
        severities(&lint(settings_for("informal"))),
        [Severity::Error]
    );
}

#[test]
fn an_explicit_rule_level_beats_the_genre() {
    let mut settings = settings_for("change-comms");
    settings.rule_levels.insert(
        id("core:docs-discipline.history-internal-reference"),
        Level::On(Severity::Error),
    );
    assert_eq!(severities(&lint(settings)), [Severity::Error]);
}

#[test]
fn an_explicit_category_level_beats_the_genre() {
    for genre in ["consumer", "change-comms"] {
        let mut settings = settings_for(genre);
        settings.categories.insert(
            CategoryId::new("docs-discipline").unwrap(),
            CategoryDials {
                level: Some(Level::On(Severity::Error)),
                ..CategoryDials::default()
            },
        );
        assert_eq!(severities(&lint(settings)), [Severity::Error], "{genre}");
    }
}

#[test]
fn a_rule_that_is_not_the_history_rule_ignores_the_genre() {
    let rules = [tokens("other", "refactored", &json!({"severity": "error"}))];
    let set = ruleset(vec![category("docs-discipline", &rules)]);
    let doc = Engine::new(vec![set], settings_for("change-comms"))
        .unwrap()
        .lint(&Input::markdown("a.md", "We refactored the parser.\n"))
        .unwrap();
    assert_eq!(severities(&doc), [Severity::Error]);
}
