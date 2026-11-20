//! Genre tolerances reach the findings: `docs-discipline.history-internal-reference` is off in
//! change-comms, a suggestion in internal and a warning in consumer and reference. Explicit
//! configuration sits above a genre default (`docs/contract/config.md`, section 1.1).

mod common;

use common::{category, id, ruleset, tokens};
use serde_json::json;
use slopvac_context::GenreAssignment;
use slopvac_engine::{CategoryDials, Engine, Input, Level, Settings};
use slopvac_model::{
    CategoryId, Document, GenreId, GenreResult, GenreSource, NonEmptyString, Severity,
};

const BODY: &str = "We refactored the parser.\n";

/// Settings that choose the genre of each document. `genre` is the run's fallback.
fn settings() -> Settings {
    let mut settings = Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new("informal").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    settings.genre_assignments = Some(vec![GenreAssignment {
        files: vec!["notes/**".to_owned()],
        genre: "internal".to_owned(),
    }]);
    settings
}

fn engine(settings: Settings) -> Engine {
    // The rule ships as an error so that any genre level shows up as a change.
    let rules = [tokens(
        "history-internal-reference",
        "refactored",
        &json!({"severity": "error"}),
    )];
    let set = ruleset(vec![category("docs-discipline", &rules)]);
    Engine::new(vec![set], settings).unwrap()
}

fn lint_genre(settings: Settings, genre: &str) -> Document {
    let source = format!("---\ngenre: {genre}\n---\n{BODY}");
    engine(settings)
        .lint(&Input::markdown("a.md", &source).with_declared_genre(genre))
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
        let doc = lint_genre(settings(), genre);
        assert_eq!(doc.genre.id.as_str(), genre);
        assert_eq!(doc.genre.source, GenreSource::Explicit);
        assert_eq!(severities(&doc), want, "{genre}");
    }
}

#[test]
fn documents_of_one_run_get_the_levels_of_their_own_genres() {
    let engine = engine(settings());
    let front = format!("---\ngenre: change-comms\n---\n{BODY}");
    let detected = format!("# Usage\n\n{BODY}");
    let inputs = [
        Input::markdown("docs/readme.md", &front).with_declared_genre("change-comms"),
        Input::markdown("docs/readme.md", &detected),
        Input::markdown("notes/a.md", BODY),
        Input::markdown("plain.md", BODY),
    ];
    let docs: Vec<Document> = engine
        .lint_all(&inputs)
        .into_iter()
        .map(Result::unwrap)
        .collect();
    let got: Vec<(&str, GenreSource, Vec<Severity>)> = docs
        .iter()
        .map(|d| (d.genre.id.as_str(), d.genre.source, severities(d)))
        .collect();
    assert_eq!(
        got,
        [
            ("change-comms", GenreSource::Explicit, vec![]),
            ("consumer", GenreSource::Detected, vec![Severity::Warning]),
            (
                "internal",
                GenreSource::Explicit,
                vec![Severity::Suggestion]
            ),
            ("consumer", GenreSource::Default, vec![Severity::Warning]),
        ]
    );
}

#[test]
fn without_per_document_genres_the_run_genre_applies_to_every_document() {
    let mut settings = settings();
    settings.genre_assignments = None;
    settings.genre = GenreResult {
        id: GenreId::new("change-comms").unwrap(),
        source: GenreSource::Explicit,
        reasons: Vec::new(),
    };
    let doc = lint_genre(settings, "consumer");
    assert_eq!(doc.genre.id.as_str(), "change-comms");
    assert_eq!(severities(&doc), []);
}

#[test]
fn a_genre_without_a_tolerance_keeps_the_shipped_severity() {
    assert_eq!(
        severities(&lint_genre(settings(), "informal")),
        [Severity::Error]
    );
}

#[test]
fn an_explicit_rule_level_beats_the_genre() {
    let mut settings = settings();
    settings.rule_levels.insert(
        id("core:docs-discipline.history-internal-reference"),
        Level::On(Severity::Error),
    );
    assert_eq!(
        severities(&lint_genre(settings, "change-comms")),
        [Severity::Error]
    );
}

#[test]
fn an_explicit_category_level_beats_the_genre() {
    for genre in ["consumer", "change-comms"] {
        let mut settings = settings();
        settings.categories.insert(
            CategoryId::new("docs-discipline").unwrap(),
            CategoryDials::default().with_level(Some(Level::On(Severity::Error))),
        );
        assert_eq!(
            severities(&lint_genre(settings, genre)),
            [Severity::Error],
            "{genre}"
        );
    }
}

#[test]
fn a_rule_that_is_not_the_history_rule_ignores_the_genre() {
    let rules = [tokens("other", "refactored", &json!({"severity": "error"}))];
    let set = ruleset(vec![category("docs-discipline", &rules)]);
    let source = format!("---\ngenre: change-comms\n---\n{BODY}");
    let doc = Engine::new(vec![set], settings())
        .unwrap()
        .lint(&Input::markdown("a.md", &source).with_declared_genre("change-comms"))
        .unwrap();
    assert_eq!(severities(&doc), [Severity::Error]);
}

#[test]
fn the_severity_shift_moves_the_level_of_the_document_genre() {
    // consumer is a warning, internal a suggestion; the shift moves one step and clamps.
    for (genre, shift, want) in [
        ("consumer", 1, Severity::Error),
        ("consumer", -1, Severity::Suggestion),
        ("internal", 1, Severity::Warning),
        ("internal", -1, Severity::Suggestion),
        ("consumer", 5, Severity::Error),
    ] {
        let mut settings = settings();
        settings.severity_shift = shift;
        assert_eq!(
            severities(&lint_genre(settings, genre)),
            [want],
            "{genre} shifted by {shift}"
        );
    }
}

#[test]
fn the_severity_shift_keeps_a_genre_that_turns_the_rule_off_off() {
    let mut settings = settings();
    settings.severity_shift = 1;
    assert_eq!(severities(&lint_genre(settings, "change-comms")), []);
}

#[test]
fn the_severity_shift_does_not_move_an_explicit_rule_level() {
    let mut settings = settings();
    settings.severity_shift = 1;
    settings.rule_levels.insert(
        id("core:docs-discipline.history-internal-reference"),
        Level::On(Severity::Warning),
    );
    assert_eq!(
        severities(&lint_genre(settings, "consumer")),
        [Severity::Warning]
    );
}
