//! Settings, rule selection, notices and parallel runs.
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

use common::{category, engine, id, places, rule, ruleset, settings, tokens};
use rstest::rstest;
use serde_json::json;
use slopvac_engine::{Engine, Input, Level, Mode};
use slopvac_model::{NoticeLevel, Severity};

fn leverage() -> serde_json::Value {
    tokens("dull", "leverage", &json!({}))
}

fn with(settings: slopvac_engine::Settings, rules: &[serde_json::Value]) -> Engine {
    Engine::new(vec![ruleset(vec![category("cat", rules)])], settings).unwrap()
}

fn lint(e: &Engine, text: &str) -> slopvac_model::Document {
    e.lint(&Input::markdown("a.md", text)).unwrap()
}

#[test]
fn a_rule_level_overrides_the_shipped_severity() {
    let mut s = settings();
    s.rule_levels
        .insert(id("core:cat.dull"), Level::On(Severity::Error));
    let doc = lint(&with(s, &[leverage()]), "Use leverage.\n");
    assert_eq!(doc.findings[0].severity, Severity::Error);
    assert_eq!(doc.errors, 1);
}

#[test]
fn a_rule_that_is_off_does_not_run() {
    let mut s = settings();
    s.rule_levels.insert(id("core:cat.dull"), Level::Off);
    assert!(
        lint(&with(s, &[leverage()]), "Use leverage.\n")
            .findings
            .is_empty()
    );
}

#[test]
fn a_category_level_applies_to_its_rules_and_a_rule_level_beats_it() {
    let mut s = settings();
    s.categories.insert(
        "cat".parse_category(),
        slopvac_engine::CategoryDials::default().with_level(Some(Level::On(Severity::Suggestion))),
    );
    s.rule_levels
        .insert(id("core:cat.keep"), Level::On(Severity::Error));
    let e = with(s, &[leverage(), tokens("keep", "utilize", &json!({}))]);
    let doc = lint(&e, "Use leverage and utilize.\n");
    let got: Vec<_> = doc
        .findings
        .iter()
        .map(|f| (f.rule_id.local().to_owned(), f.severity))
        .collect();
    assert_eq!(
        got,
        [
            ("cat.dull".to_owned(), Severity::Suggestion),
            ("cat.keep".to_owned(), Severity::Error)
        ]
    );
}

trait ParseCategory {
    fn parse_category(&self) -> slopvac_model::CategoryName;
}
impl ParseCategory for str {
    fn parse_category(&self) -> slopvac_model::CategoryName {
        slopvac_model::CategoryName::new(self).unwrap()
    }
}

#[test]
fn applies_to_limits_a_rule_to_modes_and_genres() {
    let limited = |extra| tokens("dull", "leverage", &extra);
    let text = "Use leverage.\n";
    let in_genre = limited(json!({"applies_to": {"genres": ["reference"]}}));
    let other_genre = limited(json!({"applies_to": {"genres": ["tutorial"]}}));
    let code_only = limited(json!({"applies_to": {"modes": ["code-comments"]}}));
    assert_eq!(lint(&with(settings(), &[in_genre]), text).findings.len(), 1);
    assert!(
        lint(&with(settings(), &[other_genre]), text)
            .findings
            .is_empty()
    );
    assert!(
        lint(&with(settings(), &[code_only.clone()]), text)
            .findings
            .is_empty()
    );
    let mut s = settings();
    s.mode = Mode::CodeComments;
    assert_eq!(lint(&with(s, &[code_only]), text).findings.len(), 1);
}

#[test]
fn rules_of_kinds_the_engine_does_not_run_are_reported_in_a_warning_notice() {
    let metric = rule(
        "long",
        &json!({"kind": "metric", "metric": "syllables_per_word", "threshold": 2, "comparison": "gt"}),
        &json!({"scope": "sentence"}),
    );
    let doc = lint(&engine(&[metric, leverage()]), "Use leverage.\n");
    assert_eq!(doc.unchecked.len(), 1);
    let n = &doc.unchecked[0];
    assert_eq!(n.code.as_str(), "rule-kind-not-implemented");
    assert_eq!(n.level, NoticeLevel::Warning);
    assert!(
        n.message
            .as_str()
            .starts_with("1 active metric rule(s) did not run")
    );
    assert!(
        !doc.gate.is_passed(),
        "a warning notice makes the run incomplete"
    );
}

#[rstest]
#[case::metric(
    json!({"kind": "metric", "metric": "syllables_per_word", "threshold": 2, "comparison": "gt"}),
    json!({"scope": "sentence"}),
)]
#[case::unknown_structure_check(
    json!({"kind": "structure", "check": "mystery-check"}),
    json!({}),
)]
fn rule_kind_not_implemented_lists_its_rules_in_a_warning_notice(
    #[case] matcher: serde_json::Value,
    #[case] extra: serde_json::Value,
) {
    let doc = lint(
        &engine(&[rule("pending", &matcher, &extra)]),
        "Some text.\n",
    );
    let [n] = doc.unchecked.as_slice() else {
        panic!("{:?}", doc.unchecked)
    };
    assert_eq!(n.code.as_str(), "rule-kind-not-implemented");
    assert_eq!(n.level, NoticeLevel::Warning);
    let kind = matcher["kind"].as_str().unwrap();
    assert_eq!(
        n.message.as_str(),
        format!("1 active {kind} rule(s) did not run: the engine does not run {kind} rules yet")
    );
    assert_eq!(n.data["rules"], json!(["core:cat.pending"]));
    assert!(!doc.gate.is_passed());
}

#[test]
fn judgement_rules_never_produce_findings_or_notices() {
    let judge = rule(
        "judge",
        &json!({"kind": "judgement", "question": "Is it good?"}),
        &json!({"severity": "suggestion"}),
    );
    let doc = lint(&engine(&[judge]), "Anything at all.\n");
    assert!(doc.findings.is_empty() && doc.unchecked.is_empty());
}

#[test]
fn counts_words_sentences_and_paragraphs() {
    let doc = lint(
        &engine(&[leverage()]),
        "One two three. Four five.\n\nSix.\n",
    );
    assert_eq!((doc.words, doc.sentences, doc.paragraphs), (6, 3, 2));
}

#[test]
fn lint_all_keeps_input_order_and_matches_a_sequential_run() {
    let e = engine(&[leverage()]);
    let texts: Vec<String> = (0..40)
        .map(|n| format!("{}Use leverage.\n", "Filler.\n\n".repeat(n % 5)))
        .collect();
    let inputs: Vec<Input<'_>> = texts.iter().map(|t| Input::markdown("a.md", t)).collect();
    let parallel = e.lint_all(&inputs);
    for (input, got) in inputs.iter().zip(parallel) {
        assert_eq!(got.unwrap(), e.lint(input).unwrap());
    }
}

#[test]
fn a_bad_document_is_an_error_for_that_input_only() {
    let e = engine(&[leverage()]);
    let inputs = [
        Input::mdx("bad.mdx", "Unclosed {expression\n"),
        Input::markdown("ok.md", "Use leverage.\n"),
    ];
    let results = e.lint_all(&inputs);
    assert!(results[0].is_err());
    assert_eq!(results[1].as_ref().unwrap().findings.len(), 1);
}

#[test]
fn the_report_document_carries_settings_and_scores() {
    let mut s = settings();
    s.thresholds.max_errors = Some(0);
    s.rule_levels
        .insert(id("core:cat.dull"), Level::On(Severity::Error));
    let doc = lint(&with(s, &[leverage()]), "Use leverage.\n");
    assert_eq!(doc.path.as_str(), "a.md");
    assert_eq!(doc.profile.as_str(), "normal");
    assert_eq!(doc.categories.len(), 1);
    assert_eq!(doc.gate.reasons(), ["1 error(s), limit 0"]);
    assert_eq!(places(&doc).len(), 1);
}
