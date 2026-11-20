//! A rule limited by `applies_to.genres` takes part in a document only when the document's own
//! genre is listed, in findings, notices and the choice of the vocabulary sweep owner.

#![allow(missing_docs)]

mod common;

use common::{category, rule, ruleset, tokens};
use serde_json::{Value, json};
use slopvac_context::GenreAssignment;
use slopvac_engine::{Engine, Input, Settings, matcher_entries};
use slopvac_model::{Document, GenreId, GenreResult, GenreSource, NonEmptyString};
use slopvac_rules::MatchKind;
use slopvac_terms::Blocklist;

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
            Input::markdown("a.md", &reference).with_declared_genre("reference"),
            Input::markdown("b.md", &consumer).with_declared_genre("consumer"),
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

fn vocabulary_rule(id: &str, genres: &[&str]) -> Value {
    rule(
        id,
        &json!({"kind": "vocabulary", "check": id}),
        &json!({"message": "\"{match}\" is on the blocklist.", "applies_to": {"genres": genres}}),
    )
}

#[test]
fn the_vocabulary_sweep_is_owned_by_a_rule_that_applies_to_the_document() {
    // The preferred owner applies to reference only; the other rule applies to consumer.
    let rules = [
        vocabulary_rule("word-outside-controlled-vocabulary", &["reference"]),
        vocabulary_rule("word-used-in-wrong-part-of-speech", &["consumer"]),
    ];
    let set = ruleset(vec![category("ste-words", &rules)]);
    let blocklist = Blocklist::parse(
        "b.toml",
        "toml",
        "schema_version = 3\n[[entries]]\nword = \"parser\"\npos = \"noun\"\nreason = \"r\"\n",
    )
    .unwrap();
    let engine = Engine::new(vec![set], per_document())
        .unwrap()
        .with_blocklist(&blocklist)
        .unwrap();
    let (reference, consumer) = lint_both(&engine);
    let owners = |d: &Document| -> Vec<String> {
        d.findings
            .iter()
            .map(|f| f.rule_id.as_str().to_owned())
            .collect()
    };
    assert_eq!(
        owners(&reference),
        ["core:ste-words.word-outside-controlled-vocabulary"]
    );
    assert_eq!(
        owners(&consumer),
        ["core:ste-words.word-used-in-wrong-part-of-speech"]
    );
    // One eligible rule per document: no "shared sweep" notice for either.
    assert!(
        !notice_messages(&reference)
            .iter()
            .any(|m| m.contains("sweep"))
    );
    assert!(
        !notice_messages(&consumer)
            .iter()
            .any(|m| m.contains("sweep"))
    );
}

#[test]
fn a_rule_the_genre_excludes_does_not_run_on_the_document() {
    // `^(a|aa)+$` on 64 a's and a b exhausts the PCRE2 match limit. Run on the consumer
    // document it would abort the lint, so a rule limited to reference must not run there.
    let bomb = rule(
        "bomb",
        &json!({"kind": "pattern", "pattern": "^(a|aa)+$"}),
        &json!({"applies_to": {"genres": ["reference"]}}),
    );
    let set = ruleset(vec![category("cat", &[bomb])]);
    let engine = Engine::new(vec![set], per_document()).unwrap();
    let source = format!("---\ngenre: consumer\n---\n{}b\n", "a".repeat(64));
    let doc = engine
        .lint(&Input::markdown("a.md", &source).with_declared_genre("consumer"))
        .unwrap();
    assert_eq!(doc.genre.id.as_str(), "consumer");
    assert_eq!(doc.findings.len(), 0);
}

#[test]
fn a_metric_the_genre_excludes_is_not_measured_on_the_document() {
    let metric = rule(
        "long-sentence",
        &json!({"kind": "metric", "metric": "sentence_words", "threshold": 2.0, "comparison": "gt"}),
        &json!({"scope": "sentence", "applies_to": {"genres": ["reference"]}}),
    );
    let set = ruleset(vec![category("cat", &[metric])]);
    let engine = Engine::new(vec![set], per_document()).unwrap();
    let entries = |genre: &str| {
        let before = matcher_entries(MatchKind::Metric);
        let doc = engine
            .lint(&Input::markdown("a.md", &with_genre(genre)).with_declared_genre(genre))
            .unwrap();
        (
            matcher_entries(MatchKind::Metric) - before,
            doc.findings.len(),
        )
    };
    // The reference document is measured and reported, so the counter is live.
    assert_eq!(entries("reference"), (1, 1));
    assert_eq!(entries("consumer"), (0, 0));
}

/// Lint a document of each genre and return the matcher entries of `kind` and the number of
/// findings for the reference document and the consumer document.
fn entries_for(engine: &Engine, kind: MatchKind, body: &str) -> [(usize, usize); 2] {
    ["reference", "consumer"].map(|genre| {
        let before = matcher_entries(kind);
        let source = format!("---\ngenre: {genre}\n---\n{body}");
        let doc = engine
            .lint(&Input::markdown("a.md", &source).with_declared_genre(genre))
            .unwrap();
        (matcher_entries(kind) - before, doc.findings.len())
    })
}

#[test]
fn a_term_casing_rule_the_genre_excludes_checks_nothing_on_the_document() {
    let casing = rule(
        "casing",
        &json!({"kind": "vocabulary", "check": "term-casing"}),
        &json!({"applies_to": {"genres": ["reference"]}}),
    );
    let set = ruleset(vec![category("terms", &[casing])]);
    let engine = Engine::new(vec![set], per_document()).unwrap();
    let [reference, consumer] =
        entries_for(&engine, MatchKind::Vocabulary, "Open it on Github today.\n");
    assert_eq!((reference.0 > 0, reference.1), (true, 1));
    assert_eq!(consumer, (0, 0));
}

#[test]
fn a_diagram_rule_the_genre_excludes_checks_no_diagram_in_the_document() {
    let run_on = rule(
        "label-run-on",
        &json!({"kind": "structure", "check": "diagram-label-run-on"}),
        &json!({"applies_to": {"genres": ["reference"]}}),
    );
    let set = ruleset(vec![category("diagram", &[run_on])]);
    let engine = Engine::new(vec![set], per_document()).unwrap();
    let body =
        "```mermaid\nflowchart TD\n  A[This label has far too many words in it] --> B[End]\n```\n";
    let [reference, consumer] = entries_for(&engine, MatchKind::Structure, body);
    assert_eq!((reference.0 > 0, reference.1), (true, 1));
    assert_eq!(consumer, (0, 0));
}
