//! A rule limited by `applies_to.genres` takes part in a document only when the document's own
//! genre is listed, in findings, notices and the choice of the vocabulary sweep owner.

#![allow(missing_docs)]

mod common;

use common::{category, rule, ruleset, tokens};
use serde_json::json;
use slopvac_context::GenreAssignment;
use slopvac_engine::{Engine, Input, Settings};
use slopvac_model::{Document, GenreId, GenreResult, GenreSource, NonEmptyString};

const BODY: &str = "We refactored the parser.\n";

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

fn with_genre(genre: &str) -> String {
    format!("---\ngenre: {genre}\n---\n{BODY}")
}

/// One document per genre, linted in one run.
fn lint_both(engine: &Engine) -> (Document, Document) {
    let (reference, consumer) = (with_genre("reference"), with_genre("consumer"));
    let mut docs = engine
        .lint_all(&[
            Input::markdown("a.md", &reference),
            Input::markdown("b.md", &consumer),
        ])
        .into_iter()
        .map(Result::unwrap);
    (docs.next().unwrap(), docs.next().unwrap())
}

fn notice_messages(doc: &Document) -> Vec<&str> {
    doc.unchecked.iter().map(|n| n.message.as_str()).collect()
}

#[test]
fn a_rule_limited_to_a_genre_reports_only_in_that_genre() {
    let rules = [tokens(
        "only-reference",
        "refactored",
        &json!({"applies_to": {"genres": ["reference"]}}),
    )];
    let set = ruleset(vec![category("cat", &rules)]);
    let engine = Engine::new(vec![set], per_document()).unwrap();
    let (reference, consumer) = lint_both(&engine);
    assert_eq!(reference.findings.len(), 1);
    assert_eq!(consumer.findings.len(), 0);
}

#[test]
fn a_not_run_notice_is_kept_for_a_document_of_a_listed_genre_only() {
    let rules = [rule(
        "title",
        &json!({"kind": "capitalization", "style": "title"}),
        &json!({"applies_to": {"genres": ["reference"]}}),
    )];
    let set = ruleset(vec![category("cat", &rules)]);
    let engine = Engine::new(vec![set], per_document()).unwrap();
    let (reference, consumer) = lint_both(&engine);
    let did_not_run = |d: &Document| {
        notice_messages(d)
            .iter()
            .any(|m| m.starts_with("1 active capitalization rule(s) did not run"))
    };
    assert!(did_not_run(&reference), "{:?}", notice_messages(&reference));
    assert!(!did_not_run(&consumer), "{:?}", notice_messages(&consumer));
}
