//! A rule a document's genre excludes does no matcher work on that document, whatever its
//! kind. Work is counted as matcher entries per rule kind (`test-counters`), not timed.
//!
//! `fixtures` is an exhaustive `match` on `MatchKind`: a new kind does not compile until it
//! has a fixture here, and a kind whose rules run a matcher before checking the genre fails
//! the test below, named by its kind.

#![allow(missing_docs)]

mod common;

use common::{category, rule, ruleset};
use serde_json::{Value, json};
use slopvac_context::GenreAssignment;
use slopvac_engine::{Engine, Input, Settings, matcher_entries};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString};
use slopvac_rules::MatchKind;
use slopvac_terms::Blocklist;

/// A rule of one kind, the category that holds it, the body of a document it reports on, and
/// a blocklist for the rules that need one.
struct Fixture {
    category: &'static str,
    rule: Value,
    body: &'static str,
    blocklist: Option<&'static str>,
}

fn fixture(category: &'static str, matcher: &Value, extra: &Value, body: &'static str) -> Fixture {
    Fixture {
        category,
        rule: rule("r", matcher, extra),
        body,
        blocklist: None,
    }
}

/// The fixtures of `kind`, one for each way the engine runs a rule of it.
fn fixtures(kind: MatchKind) -> Vec<Fixture> {
    const PROSE: &str = "We refactored the parser.\n";
    let none = json!({});
    match kind {
        MatchKind::Tokens => vec![fixture(
            "cat",
            &json!({"kind": "tokens", "tokens": ["refactored"]}),
            &none,
            PROSE,
        )],
        MatchKind::Pattern => vec![fixture(
            "cat",
            &json!({"kind": "pattern", "pattern": "refactor\\w+"}),
            &none,
            PROSE,
        )],
        MatchKind::Substitution => vec![fixture(
            "cat",
            &json!({"kind": "substitution", "substitutions": {"refactored": "rewrote"}}),
            &none,
            PROSE,
        )],
        MatchKind::Metric => vec![fixture(
            "cat",
            &json!({
                "kind": "metric", "metric": "sentence_words",
                "threshold": 2.0, "comparison": "gt"
            }),
            &json!({"scope": "sentence"}),
            PROSE,
        )],
        MatchKind::Structure => vec![fixture(
            "diagram",
            &json!({"kind": "structure", "check": "diagram-label-run-on"}),
            &none,
            "```mermaid\nflowchart TD\n  A[This label has far too many words in it] --> B[End]\n```\n",
        )],
        MatchKind::Capitalization => vec![fixture(
            "cat",
            &json!({"kind": "capitalization", "style": "sentence"}),
            &json!({"scope": "heading"}),
            "# Configure The Server\n",
        )],
        // The term-casing check and the blocklist sweep are two matchers of one kind.
        MatchKind::Vocabulary => vec![
            fixture(
                "terms",
                &json!({"kind": "vocabulary", "check": "term-casing"}),
                &none,
                "Open it on Github today.\n",
            ),
            Fixture {
                blocklist: Some(
                    "schema_version = 3\n[[entries]]\nword = \"parser\"\npos = \"noun\"\nreason = \"r\"\n",
                ),
                ..fixture(
                    "ste-words",
                    &json!({"kind": "vocabulary", "check": "word-outside-controlled-vocabulary"}),
                    &none,
                    PROSE,
                )
            },
        ],
        // A judgement is a question for a reviewer: the engine has no matcher for it.
        MatchKind::Judgement => Vec::new(),
    }
}

/// A distinct number for each kind. The exhaustive `match` makes a new kind fail to compile
/// until it is numbered, and `every_kind_is_listed` then fails until the list has it.
const fn ordinal(kind: MatchKind) -> usize {
    match kind {
        MatchKind::Tokens => 0,
        MatchKind::Pattern => 1,
        MatchKind::Substitution => 2,
        MatchKind::Metric => 3,
        MatchKind::Structure => 4,
        MatchKind::Capitalization => 5,
        MatchKind::Vocabulary => 6,
        MatchKind::Judgement => 7,
    }
}

const ALL: [MatchKind; 8] = [
    MatchKind::Tokens,
    MatchKind::Pattern,
    MatchKind::Substitution,
    MatchKind::Metric,
    MatchKind::Structure,
    MatchKind::Capitalization,
    MatchKind::Vocabulary,
    MatchKind::Judgement,
];

fn per_document() -> Settings {
    let mut settings = Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new("informal").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    settings.genre_assignments = Some(Vec::<GenreAssignment>::new());
    settings
}

/// The matcher entries of `kind` while linting a `genre` document with `body`.
fn entries(engine: &Engine, kind: MatchKind, genre: &str, body: &str) -> usize {
    let before = matcher_entries(kind);
    let source = format!("---\ngenre: {genre}\n---\n{body}");
    engine.lint(&Input::markdown("a.md", &source)).unwrap();
    matcher_entries(kind) - before
}

#[test]
fn every_kind_is_listed() {
    let mut ordinals: Vec<usize> = ALL.iter().map(|k| ordinal(*k)).collect();
    ordinals.sort_unstable();
    assert_eq!(ordinals, (0..ALL.len()).collect::<Vec<_>>());
}

#[test]
fn a_rule_the_genre_excludes_does_no_matcher_work_for_any_kind() {
    let mut wrong = Vec::new();
    for kind in ALL {
        for (n, f) in fixtures(kind).into_iter().enumerate() {
            let mut limited = f.rule.clone();
            limited["applies_to"] = json!({"genres": ["reference"]});
            let set = ruleset(vec![category(f.category, &[limited])]);
            let mut engine = Engine::new(vec![set], per_document()).unwrap();
            if let Some(text) = f.blocklist {
                let list = Blocklist::parse("b.toml", "toml", text).unwrap();
                engine = engine.with_blocklist(&list).unwrap();
            }
            let eligible = entries(&engine, kind, "reference", f.body);
            let excluded = entries(&engine, kind, "consumer", f.body);
            if eligible == 0 {
                wrong.push(format!(
                    "{kind:?} #{n}: no matcher work on an eligible document"
                ));
            }
            if excluded != 0 {
                wrong.push(format!(
                    "{kind:?} #{n}: {excluded} matcher entries on an excluded document"
                ));
            }
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}
