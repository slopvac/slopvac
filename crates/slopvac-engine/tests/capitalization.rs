//! The `capitalization` rule kind, in the sentence-case style: the 2.x `title-case-heading`
//! logic. Expected results come from the 2.x pattern rule run on the same headings.

#![allow(missing_docs, clippy::assert_is_empty)]

mod common;

use common::{category, id, places, rule, ruleset, settings};
use serde_json::{Value, json};
use slopvac_engine::{Engine, Input};
use slopvac_model::NoticeLevel;

fn sentence_rule(allowed: &[&str]) -> Value {
    rule(
        "title-case",
        &json!({"kind": "capitalization", "style": "sentence", "allowed_words": allowed}),
        &json!({"scope": "heading", "message": "title-case heading: {match}"}),
    )
}

fn engine_with(matcher: &Value, locale_allow: &[&str]) -> Engine {
    let mut s = settings();
    s.allowed_words = locale_allow.iter().map(|w| (*w).to_owned()).collect();
    Engine::new(vec![ruleset(vec![category("cat", &[matcher.clone()])])], s).unwrap()
}

fn flagged(engine: &Engine, source: &str) -> Vec<String> {
    engine
        .lint(&Input::markdown("a.md", source))
        .unwrap()
        .findings
        .into_iter()
        .map(|f| f.matched_text)
        .collect()
}

#[test]
fn title_case_headings_are_flagged() {
    let e = engine_with(&sentence_rule(&[]), &[]);
    for heading in [
        "Configure The Server",
        "Getting Started",
        "Table of Contents",
        "Step 1: Check Replication Lag",
        "Using The JSON Flag",
        "Tips & Tricks For You",
        "Install The Plugin, Then Run",
        "The Quick Brown",
        "Foo Bar:",
        "Foo-Bar Baz",
        "Foo's Bar",
        "Hello World 2.0 Release",
        "Install The  Plugin",
    ] {
        assert_eq!(
            flagged(&e, &format!("# {heading}\n")),
            [heading],
            "{heading}"
        );
    }
}

#[test]
fn headings_that_are_sentence_case_or_hold_identifiers_pass() {
    let e = engine_with(&sentence_rule(&[]), &[]);
    for heading in [
        "Install the plugin from source",
        "Getting started",
        "Installation",
        "Use the TypeScript SDK",
        "Implement EventLog",
        "Getting Started With gRPC",
        "Set Up macOS",
        "Hello World v2",
        "Hello World (Beta)",
        "Using The `--json` Flag",
        "Configure The `x` Server",
        "Déjà Vu Time",
        "A B",
    ] {
        assert_eq!(
            flagged(&e, &format!("# {heading}\n")),
            Vec::<String>::new(),
            "{heading}"
        );
    }
}

#[test]
fn only_headings_are_read() {
    let e = engine_with(&sentence_rule(&[]), &[]);
    assert!(flagged(&e, "Configure The Server\n").is_empty());
}

#[test]
fn a_finding_reports_the_heading_text_and_its_position() {
    let e = engine_with(&sentence_rule(&[]), &[]);
    let doc = e
        .lint(&Input::markdown(
            "a.md",
            "Intro.\n\n## Configure The Server\n",
        ))
        .unwrap();
    assert_eq!(places(&doc), [("core:cat.title-case".to_owned(), 3, 4)]);
    let f = &doc.findings[0];
    assert_eq!(f.rule_id, id("core:cat.title-case"));
    assert_eq!(
        f.message.as_str(),
        "title-case heading: Configure The Server"
    );
    let span = f.extent.unwrap().span;
    assert_eq!((span.start(), span.end()), (11, 31));
}

#[test]
fn allowed_words_of_the_rule_do_not_count_as_capitalised_words() {
    let plain = engine_with(&sentence_rule(&[]), &[]);
    let allowed = engine_with(&sentence_rule(&["Brave", "Search"]), &[]);
    let heading = "# Configure Brave Search\n";
    assert_eq!(flagged(&plain, heading), ["Configure Brave Search"]);
    assert!(flagged(&allowed, heading).is_empty());
}

#[test]
fn locale_allow_words_are_exempt_whatever_their_case() {
    let heading = "# Configure Brave Search\n";
    let e = engine_with(&sentence_rule(&[]), &["brave", "SEARCH"]);
    assert!(flagged(&e, heading).is_empty());
}

#[test]
fn one_allowed_word_leaves_the_other_plain_words_counting() {
    let e = engine_with(&sentence_rule(&["Server"]), &[]);
    assert_eq!(
        flagged(&e, "# Configure The Server\n"),
        ["Configure The Server"]
    );
}

#[test]
fn styles_other_than_sentence_are_reported_as_not_run() {
    let title = rule(
        "title",
        &json!({"kind": "capitalization", "style": "title"}),
        &json!({"scope": "heading"}),
    );
    let doc = engine_with(&title, &[])
        .lint(&Input::markdown("a.md", "# Configure The Server\n"))
        .unwrap();
    assert!(doc.findings.is_empty());
    assert_eq!(doc.unchecked.len(), 1);
    let n = &doc.unchecked[0];
    assert_eq!(n.code.as_str(), "rule-kind-not-implemented");
    assert_eq!(n.level, NoticeLevel::Info);
    assert!(
        n.message
            .as_str()
            .starts_with("1 active capitalization (title style) rule(s) did not run")
    );
}
