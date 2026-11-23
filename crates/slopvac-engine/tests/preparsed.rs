//! A document read once, `Engine::preparse`, serves `Engine::detect_genre` and `Engine::lint`
//! with one answer, over the engine's own genres, and `lint` refuses a read that belongs to
//! other input.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

use rstest::rstest;
use slopvac_context::{GenreAssignment, GenreRegistry, parse_genres};
use slopvac_engine::{Engine, EngineError, Input, Kind, Settings};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString};

/// A genre that only a plugin supplies: any file under `plugin/` is one.
const PLUGIN_GENRE: &str = "- schema_version: 3
  id: plugin-notes
  description: Notes that a plugin genre claims by path.
  detect:
    min_score: 1
    signals:
      - { kind: path, glob: '**/plugin/**', weight: 5 }
";

const BODY: &str = "# Notes\n\nWe refactored the parser.\n";

fn per_document() -> Settings {
    let mut settings = Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new("plugin-notes").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    settings.genre_assignments = Some(Vec::<GenreAssignment>::new());
    settings
}

fn engine() -> Engine {
    Engine::new(Vec::new(), per_document()).expect("the engine builds")
}

fn plugin_registry() -> GenreRegistry {
    GenreRegistry::new(parse_genres(PLUGIN_GENRE).unwrap(), "plugin-notes").unwrap()
}

#[test]
fn a_plugin_genre_wins_detection_and_the_lint_reports_the_same_genre() {
    let engine = engine()
        .with_genres(plugin_registry())
        .expect("assignments are set");
    let input = Input::new("plugin/a.md", BODY);
    let pre = engine.preparse(&input).expect("the document reads");

    let detected = engine
        .detect_genre(&input, &pre)
        .expect("detection works")
        .expect("a Markdown file has a genre");
    let linted = engine
        .lint(&input.clone().with_preparsed(&pre))
        .expect("the lint works");

    assert_eq!(detected.id.as_str(), "plugin-notes");
    assert_eq!(detected.source, GenreSource::Detected);
    assert_eq!(linted.genre, detected);
}

#[test]
fn without_per_document_genres_the_engine_refuses_a_genre_registry() {
    let mut settings = per_document();
    settings.genre_assignments = None;
    let engine = Engine::new(Vec::new(), settings).expect("the engine builds");
    assert!(matches!(
        engine.with_genres(plugin_registry()),
        Err(EngineError::Internal(_))
    ));
}

/// A read made from `read` is refused for the input `used`, in a lint and in a detection.
#[rstest]
#[case::other_text_of_the_same_length(
    "# Notes\n\nWe refactored the parser.\n",
    "# Notes\n\nWe refactored the parsed.\n",
    Kind::Markdown
)]
#[case::longer_text(BODY, "# Notes\n\nWe refactored the parser. Twice.\n", Kind::Markdown)]
#[case::other_kind(BODY, BODY, Kind::Mdx)]
fn a_read_of_other_input_is_refused(#[case] read: &str, #[case] used: &str, #[case] kind: Kind) {
    let engine = engine();
    let pre = engine
        .preparse(&Input::new("a.md", read))
        .expect("the document reads");
    let input = Input::new("a.md", used).with_kind(kind);

    assert!(matches!(
        engine.lint(&input.clone().with_preparsed(&pre)),
        Err(EngineError::Internal(_))
    ));
    assert!(matches!(
        engine.detect_genre(&input, &pre),
        Err(EngineError::Internal(_))
    ));
}

#[test]
fn a_read_of_the_same_input_is_accepted() {
    let engine = engine();
    let input = Input::new("a.md", BODY);
    let pre = engine.preparse(&input).expect("the document reads");
    let with = engine
        .lint(&input.clone().with_preparsed(&pre))
        .expect("the lint works");
    let without = engine.lint(&input).expect("the lint works");
    assert_eq!(with.findings, without.findings);
    assert_eq!(with.genre, without.genre);
}
