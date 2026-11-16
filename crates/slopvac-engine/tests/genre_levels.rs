//! Genre tolerances reach the engine settings: `docs-discipline.history-internal-reference`
//! is off in change-comms, a suggestion in internal and a warning in consumer and reference.

use slopvac_context::builtin_genres;
use slopvac_engine::{Level, Settings};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString, RuleId, Severity};

fn settings_for(genre: &str) -> Settings {
    Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new(genre).unwrap(),
            source: GenreSource::Explicit,
            reasons: Vec::new(),
        },
    )
}

fn history_rule() -> RuleId {
    RuleId::new("core:docs-discipline.history-internal-reference").unwrap()
}

#[test]
fn each_genre_sets_its_level_for_the_history_rule() {
    let registry = builtin_genres().unwrap();
    for (genre, want) in [
        ("change-comms", Level::Off),
        ("internal", Level::On(Severity::Suggestion)),
        ("consumer", Level::On(Severity::Warning)),
        ("reference", Level::On(Severity::Warning)),
    ] {
        let mut settings = settings_for(genre);
        settings.apply_genre_levels(&registry).unwrap();
        assert_eq!(
            settings.rule_levels.get(&history_rule()),
            Some(&want),
            "{genre}"
        );
    }
}

#[test]
fn a_genre_without_a_tolerance_leaves_the_rule_alone() {
    let registry = builtin_genres().unwrap();
    let mut settings = settings_for("informal");
    settings.apply_genre_levels(&registry).unwrap();
    assert!(settings.rule_levels.is_empty());
}

#[test]
fn a_level_set_above_the_genre_wins() {
    let registry = builtin_genres().unwrap();
    let mut settings = settings_for("change-comms");
    settings
        .rule_levels
        .insert(history_rule(), Level::On(Severity::Error));
    settings.apply_genre_levels(&registry).unwrap();
    assert_eq!(
        settings.rule_levels.get(&history_rule()),
        Some(&Level::On(Severity::Error))
    );
}
