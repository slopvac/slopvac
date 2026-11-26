//! The `capitalization` rule kind, in the sentence-case style: the 2.x `title-case-heading`
//! logic. Expected results come from the 2.x pattern rule run on the same headings.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(
    clippy::allow_attributes_without_reason,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]
#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

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

/// A rule that reads each source line as one text, so backticks reach the matcher as written.
fn raw_rule(allowed: &[&str]) -> Value {
    rule(
        "title-case",
        &json!({"kind": "capitalization", "style": "sentence", "allowed_words": allowed}),
        &json!({"scope": "raw"}),
    )
}

fn raw_flagged(engine: &Engine, lines: &[&str]) -> Vec<String> {
    let source = lines.join("\n") + "\n";
    flagged(engine, &source)
}

#[test]
fn backtick_spans_with_spaces_are_one_word() {
    // Expected from the 2.x regex: both lines match.
    let e = engine_with(&raw_rule(&[]), &[]);
    let lines = ["`Alpha Beta` Gamma", "`x` Server Beta"];
    assert_eq!(raw_flagged(&e, &lines), lines);
}

#[test]
fn an_unclosed_or_glued_backtick_is_not_a_code_span() {
    // Expected from the 2.x regex: neither line matches.
    let e = engine_with(&raw_rule(&[]), &[]);
    assert!(raw_flagged(&e, &["Foo Bar `Baz", "Foo Bar `a`b"]).is_empty());
}

#[test]
fn a_code_span_may_carry_trailing_punctuation() {
    let e = engine_with(&raw_rule(&[]), &[]);
    assert_eq!(raw_flagged(&e, &["Foo Bar `a`,"]), ["Foo Bar `a`,"]);
}

#[test]
fn decimal_digits_of_any_script_are_numbers() {
    // Expected from the 2.x regex, whose `\d` is Unicode: both lines match.
    let e = engine_with(&raw_rule(&[]), &[]);
    let lines = ["Release Notes \u{662}", "Release Notes \u{663}.\u{664}"];
    assert_eq!(raw_flagged(&e, &lines), lines);
}

/// The 2.x `title-case-heading` regex decided each case of the fixture, with the same allowed
/// words injected (`fixtures/generate_title_case.py`).
#[test]
fn capitalization_matches_the_2x_regex_on_generated_headings() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/title-case-differential.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let allowed: Vec<&str> = fixture["allowed_words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    let cases: Vec<(&str, bool)> = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c[0].as_str().unwrap(), c[1].as_bool().unwrap()))
        .collect();
    assert!(cases.iter().filter(|(_, m)| *m).count() > 100);
    let e = engine_with(&raw_rule(&allowed), &[]);
    let source = cases.iter().map(|(t, _)| *t).collect::<Vec<_>>().join("\n") + "\n";
    let doc = e.lint(&Input::markdown("a.md", &source)).unwrap();
    let got: std::collections::BTreeSet<usize> =
        doc.findings.iter().map(|f| f.start.line() - 1).collect();
    let wrong: Vec<String> = cases
        .iter()
        .enumerate()
        .filter(|(i, (_, want))| got.contains(i) != *want)
        .map(|(_, (t, want))| format!("{t:?}: 2.x says {want}"))
        .collect();
    assert!(
        wrong.is_empty(),
        "{} differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn styles_other_than_sentence_leave_the_check_incomplete() {
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
    assert_eq!(n.code.as_str(), "rule-style-not-implemented");
    assert_eq!(n.level, NoticeLevel::Warning);
    assert!(
        n.message
            .as_str()
            .starts_with("1 active capitalization rule(s) did not run")
    );
    assert_eq!(n.data["rules"], json!(["core:cat.title"]));
    assert!(!doc.gate.is_passed());
    assert_eq!(doc.gate.reasons().len(), 1);
    assert!(doc.gate.reasons()[0].starts_with("incomplete check:"));
}

#[test]
fn escaped_backticks_in_a_markdown_heading_are_literal_text() {
    // Expected from the 2.x engine run on this source: lines 1 and 3 are reported, line 5 is not.
    let e = engine_with(&sentence_rule(&[]), &[]);
    let source =
        "# \\`Alpha Beta\\` Gamma\n\n## \\`Alpha\\` Beta Gamma\n\n## Foo \\`bar Baz\\` qux\n";
    let doc = e.lint(&Input::markdown("a.md", source)).unwrap();
    let got: Vec<(usize, &str)> = doc
        .findings
        .iter()
        .map(|f| (f.start.line(), f.matched_text.as_str()))
        .collect();
    assert_eq!(got, [(1, "`Alpha Beta` Gamma"), (3, "`Alpha` Beta Gamma")]);
}

#[test]
fn the_unrun_style_notice_lists_its_rule_ids_in_sorted_order() {
    let styled = |id: &str| {
        rule(
            id,
            &json!({"kind": "capitalization", "style": "title"}),
            &json!({"scope": "heading"}),
        )
    };
    let e = Engine::new(
        vec![ruleset(vec![category(
            "cat",
            &[styled("zeta"), styled("alpha")],
        )])],
        settings(),
    )
    .unwrap();
    let doc = e.lint(&Input::markdown("a.md", "# Heading\n")).unwrap();
    let [n] = doc.unchecked.as_slice() else {
        panic!("{:?}", doc.unchecked)
    };
    assert_eq!(n.data["rules"], json!(["core:cat.alpha", "core:cat.zeta"]));
}
