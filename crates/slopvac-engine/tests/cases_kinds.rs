//! The case runner checks the examples of every mechanically checkable rule kind and
//! sets its exit code by the contract ruling in `docs/contract/rules.md`.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

mod common;

use common::{engine, id, rule, tokens};
use rstest::rstest;
use serde_json::{Value, json};
use slopvac_engine::Engine;
use slopvac_engine::cases::{CaseReport, Origin, Outcome, run_cases};
use slopvac_model::NoticeLevel;
use slopvac_terms::Blocklist;

/// A rule of `kind` whose only check is one example pair.
fn example_rule(kind: &str, bad: &str, good: &str) -> Value {
    let examples = json!({"examples": [{"bad": bad, "good": good}]});
    match kind {
        "metric" => rule(
            "m",
            &json!({"kind": "metric", "metric": "sentence_words", "threshold": 3, "comparison": "gt"}),
            &examples,
        ),
        "capitalization" => {
            let mut extra = examples;
            extra["scope"] = json!("heading");
            rule(
                "c",
                &json!({"kind": "capitalization", "style": "sentence"}),
                &extra,
            )
        }
        "vocabulary" => rule(
            "v",
            &json!({"kind": "vocabulary", "check": "word-outside-controlled-vocabulary"}),
            &examples,
        ),
        "structure" => {
            let mut extra = examples;
            extra["scope"] = json!("document");
            rule(
                "s",
                &json!({"kind": "structure", "check": "heading-hierarchy"}),
                &extra,
            )
        }
        other => unreachable!("{other}"),
    }
}

fn blocklist() -> Blocklist {
    Blocklist::parse(
        "b.toml",
        "toml",
        "schema_version = 3\n[[entries]]\nword = \"deploy\"\npos = \"noun\"\nreason = \"r\"\n",
    )
    .unwrap()
}

fn run(rules: &[Value]) -> CaseReport {
    run_cases(&engine(rules).with_blocklist(&blocklist()).unwrap(), None).unwrap()
}

fn flags(report: &CaseReport) -> Vec<(Origin, bool)> {
    report
        .results
        .iter()
        .map(|r| (r.origin, r.outcome == Outcome::Passed))
        .collect()
}

/// One kind with a text that the rule flags and a text it leaves alone.
const KINDS: [(&str, &str, &str); 4] = [
    ("metric", "One two three four five six.", "One two."),
    ("capitalization", "Install The Plugin", "Install the plugin"),
    ("vocabulary", "The deploy failed.", "We deploy the app."),
    ("structure", "# One\n\n#### Four\n", "# One\n\n## Two\n"),
];

#[rstest]
#[case::metric(0)]
#[case::capitalization(1)]
#[case::vocabulary(2)]
#[case::structure(3)]
fn a_passing_example_pair_passes(#[case] kind: usize) {
    let (name, bad, good) = KINDS[kind];
    let report = run(&[example_rule(name, bad, good)]);
    assert_eq!(
        flags(&report),
        [
            (Origin::ExampleBad(0), true),
            (Origin::ExampleGood(0), true)
        ],
        "{report}"
    );
    assert_eq!(report.exit_code(), 0, "{report}");
}

#[rstest]
#[case::metric(0)]
#[case::capitalization(1)]
#[case::vocabulary(2)]
#[case::structure(3)]
fn a_failing_example_pair_fails_and_exits_1(#[case] kind: usize) {
    let (name, bad, good) = KINDS[kind];
    // Swapped: the bad example is not flagged and the good one is.
    let report = run(&[example_rule(name, good, bad)]);
    assert_eq!(
        flags(&report),
        [
            (Origin::ExampleBad(0), false),
            (Origin::ExampleGood(0), false)
        ],
        "{report}"
    );
    assert_eq!(report.exit_code(), 1, "{report}");
}

fn failing_foo() -> Value {
    tokens("r", "foo", &json!({"cases": [{"input": "foo", "want": 2}]}))
}

fn passing_foo() -> Value {
    tokens("r", "foo", &json!({"cases": [{"input": "foo", "want": 1}]}))
}

/// A rule with nothing to run, so the runner cannot check it.
fn unchecked() -> Value {
    tokens("bare", "bar", &json!({}))
}

fn judgement() -> Value {
    rule(
        "j",
        &json!({"kind": "judgement", "question": "Is it vague?"}),
        &json!({"severity": "suggestion", "examples": [{"bad": "foo", "good": "foo"}]}),
    )
}

#[test]
fn a_failure_outranks_an_unchecked_rule_and_the_notice_still_names_it() {
    let report = run(&[failing_foo(), unchecked()]);
    assert_eq!(report.exit_code(), 1);
    let notice = report.unchecked_notice().unwrap();
    assert_eq!(notice.data["rules"], json!(["core:cat.bare"]));
}

#[test]
fn an_unchecked_rule_alone_exits_2_with_the_warning_notice() {
    let report = run(&[passing_foo(), unchecked()]);
    assert_eq!(report.exit_code(), 2);
    let notice = report.unchecked_notice().unwrap();
    assert_eq!(notice.code.as_str(), "cases-rule-unchecked");
    assert_eq!(notice.level, NoticeLevel::Warning);
    assert_eq!(notice.data["rules"], json!(["core:cat.bare"]));
}

#[test]
fn a_clean_run_exits_0_without_a_notice() {
    let report = run(&[passing_foo()]);
    assert_eq!(report.exit_code(), 0);
    assert!(report.unchecked_notice().is_none());
}

#[test]
fn a_judgement_rule_never_changes_the_exit_code() {
    let report = run(&[passing_foo(), judgement()]);
    assert_eq!(report.results.len(), 1, "{report}");
    assert_eq!(report.exit_code(), 0);
    assert!(report.unchecked_notice().is_none());
    let failing = run(&[failing_foo(), judgement()]);
    assert_eq!(failing.exit_code(), 1);
}

#[test]
fn a_kind_the_engine_cannot_run_is_unchecked_not_passed() {
    let title = rule(
        "c",
        &json!({"kind": "capitalization", "style": "title"}),
        &json!({"scope": "heading", "examples": [{"bad": "install the plugin", "good": "Install The Plugin"}]}),
    );
    let report = run(&[title]);
    assert_eq!(report.exit_code(), 2, "{report}");
    assert_eq!(
        report.unchecked_notice().unwrap().data["rules"],
        json!(["core:cat.c"])
    );
}

#[test]
fn a_switched_off_rule_is_not_active_so_it_is_not_unchecked() {
    // A title-style capitalization rule is a kind the runner cannot check, but it is off.
    let title = rule(
        "c",
        &json!({"kind": "capitalization", "style": "title"}),
        &json!({"scope": "heading", "examples": [{"bad": "install the plugin", "good": "Install The Plugin"}]}),
    );
    let set = common::ruleset(vec![common::category("cat", &[passing_foo(), title])]);
    let mut settings = common::settings();
    settings
        .rule_levels
        .insert(id("core:cat.c"), slopvac_engine::Level::Off);
    let report = run_cases(&Engine::new(vec![set], settings).unwrap(), None).unwrap();
    assert_eq!(report.exit_code(), 0, "{report}");
    assert!(report.unchecked_notice().is_none());
}

#[test]
fn selecting_only_a_judgement_rule_is_an_empty_run_that_exits_0() {
    let e: Engine = engine(&[judgement()]);
    let report = run_cases(&e, Some(&id("core:cat.j"))).unwrap();
    assert!(report.results.is_empty(), "{report}");
    assert_eq!(report.exit_code(), 0);
    assert!(report.unchecked_notice().is_none());
}

#[test]
fn an_unknown_selected_rule_is_still_an_error() {
    let err = run_cases(&engine(&[judgement()]), Some(&id("core:cat.nope"))).unwrap_err();
    assert_eq!(err.to_string(), "no rule core:cat.nope");
}
