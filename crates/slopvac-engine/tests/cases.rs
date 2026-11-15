//! The case runner on synthetic rules and on the shipped core rules.

#![allow(missing_docs)]

mod common;

use std::path::PathBuf;

use common::{engine, id, ruleset, settings, tokens};
use serde_json::{Value, json};
use slopvac_engine::cases::{CaseReport, Origin, Outcome, coverage, run_cases};
use slopvac_engine::{Engine, Input, Level};
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, RuleSet};

/// A tokens rule `cat.r` that matches "foo" with the given `cases` and `examples`.
fn foo_rule(extra: &Value) -> Value {
    tokens("r", "foo", extra)
}

fn run(extra: &Value) -> CaseReport {
    run_cases(&engine(&[foo_rule(extra)]), None).unwrap()
}

fn outcomes(report: &CaseReport) -> Vec<(Origin, bool)> {
    report
        .results
        .iter()
        .map(|r| (r.origin, r.outcome == Outcome::Passed))
        .collect()
}

#[test]
fn want_counts_the_findings_of_the_rule() {
    let report = run(&json!({"cases": [
        {"input": "foo and foo", "want": 2},
        {"input": "foo and foo", "want": 1},
        {"input": "bar", "want": 0},
    ]}));
    assert_eq!(
        outcomes(&report),
        [
            (Origin::Case(0), true),
            (Origin::Case(1), false),
            (Origin::Case(2), true)
        ]
    );
}

#[test]
fn want_range_is_inclusive_and_either_bound_may_be_open() {
    let report = run(&json!({"cases": [
        {"input": "foo foo", "want": {"min": 2, "max": 3}},
        {"input": "foo foo foo", "want": {"min": 2, "max": 3}},
        {"input": "foo foo foo foo", "want": {"min": 2, "max": 3}},
        {"input": "foo", "want": {"min": 2}},
        {"input": "foo foo foo foo foo", "want": {"max": 4}},
    ]}));
    let passed: Vec<bool> = outcomes(&report).iter().map(|o| o.1).collect();
    assert_eq!(passed, [true, true, false, false, false]);
}

#[test]
fn contains_and_absent_read_the_matched_text_and_the_message() {
    let report = run(&json!({"cases": [
        {"input": "a foo b", "contains": ["foo"]},
        {"input": "a foo b", "contains": ["Found"]},
        {"input": "a foo b", "contains": ["nope"]},
        {"input": "a foo b", "absent": ["nope"]},
        {"input": "a foo b", "absent": ["foo"]},
    ]}));
    let passed: Vec<bool> = outcomes(&report).iter().map(|o| o.1).collect();
    assert_eq!(passed, [true, true, false, true, false]);
}

#[test]
fn a_failure_says_what_was_expected_and_found() {
    let report = run(&json!({"cases": [{"input": "foo", "want": 2}]}));
    assert_eq!(
        report.to_string(),
        "core:cat.r cases[0]: want 2 finding(s), got 1\n0 passed, 1 failed, 0 skipped"
    );
    assert_eq!(report.failing_rules(), [&id("core:cat.r")]);
}

#[test]
fn a_mechanical_rule_examples_are_cases() {
    let report = run(&json!({"examples": [
        {"bad": "use foo", "good": "use bar"},
        {"bad": "foo"},
    ]}));
    assert_eq!(
        outcomes(&report),
        [
            (Origin::ExampleBad(0), true),
            (Origin::ExampleGood(0), true),
            (Origin::ExampleBad(1), true),
        ]
    );
}

#[test]
fn a_bad_example_the_engine_does_not_flag_fails() {
    // The pattern matches the bad text, but the engine skips list items for this rule.
    let report = run(&json!({
        "exclude_blocks": ["list-item"],
        "examples": [{"bad": "- foo"}],
    }));
    assert_eq!(outcomes(&report), [(Origin::ExampleBad(0), false)]);
}

#[test]
fn a_heading_scoped_example_is_linted_as_a_heading() {
    let heading = common::rule(
        "h",
        &json!({"kind": "pattern", "pattern": "^Foo"}),
        &json!({"scope": "heading", "examples": [{"bad": "Foo bar", "good": "Bar foo"}]}),
    );
    let report = run_cases(&engine(&[heading]), None).unwrap();
    assert_eq!(report.passed(), 2, "{report}");
}

#[test]
fn a_judgement_rule_examples_are_documentation_only() {
    let judgement = common::rule(
        "j",
        &json!({"kind": "judgement", "question": "Is it vague?"}),
        &json!({"severity": "suggestion", "examples": [{"bad": "foo", "good": "foo"}]}),
    );
    let report = run_cases(&engine(&[judgement]), None).unwrap();
    assert_eq!(report.results.len(), 0);
}

#[test]
fn only_the_findings_of_the_rule_under_test_count() {
    let report = run_cases(
        &engine(&[
            foo_rule(&json!({"cases": [{"input": "foo bar", "want": 1}]})),
            tokens("other", "bar", &json!({})),
        ]),
        None,
    )
    .unwrap();
    assert!(report.is_clean(), "{report}");
    assert_eq!(report.passed(), 1);
}

#[test]
fn a_rule_the_engine_does_not_run_is_skipped_not_passed() {
    let set = ruleset(vec![common::category(
        "cat",
        &[foo_rule(&json!({"cases": [{"input": "foo", "want": 1}]}))],
    )]);
    let mut s = settings();
    s.rule_levels.insert(id("core:cat.r"), Level::Off);
    let report = run_cases(&Engine::new(vec![set], s).unwrap(), None).unwrap();
    assert_eq!((report.passed(), report.skipped()), (0, 1));
    assert!(report.is_clean());
}

#[test]
fn only_selects_one_rule() {
    let e = engine(&[
        foo_rule(&json!({"cases": [{"input": "bar", "want": 1}]})),
        tokens(
            "ok",
            "bar",
            &json!({"cases": [{"input": "bar", "want": 1}]}),
        ),
    ]);
    let all = run_cases(&e, None).unwrap();
    let one = run_cases(&e, Some(&id("core:cat.ok"))).unwrap();
    assert_eq!((all.results.len(), one.results.len()), (2, 1));
    assert!(one.is_clean());
}

#[test]
fn without_sets_known_failures_aside() {
    let report = run(&json!({"cases": [{"input": "foo", "want": 2}]}));
    assert!(!report.is_clean());
    assert!(report.without(&["core:cat.r"]).is_clean());
}

#[test]
fn coverage_flags_rules_that_match_nothing_in_the_corpus() {
    let e = engine(&[
        foo_rule(&json!({})),
        tokens("never", "zzz", &json!({})),
        common::rule(
            "j",
            &json!({"kind": "judgement", "question": "Is it vague?"}),
            &json!({"severity": "suggestion"}),
        ),
    ]);
    let corpus = [
        Input::markdown("a.md", "foo here"),
        Input::markdown("b.md", "foo and foo"),
    ];
    let cov = coverage(&e, &corpus).unwrap();
    assert_eq!(cov.documents, 2);
    assert_eq!(cov.findings[&id("core:cat.r")], 3);
    assert_eq!(cov.unmatched().collect::<Vec<_>>(), [&id("core:cat.never")]);
    assert_eq!(cov.findings.len(), 2, "judgement rules are not mechanical");
    assert_eq!(
        cov.to_string(),
        "core:cat.never: matched nothing\n1 of 2 rules matched in 2 document(s); 1 matched nothing"
    );
}

// ---- the shipped core rules ----

fn core_engine() -> Engine {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../slopvac-rules/tests/fixtures/core-v3");
    let set = RuleSet::load_dir(
        &PackId::new("core").unwrap(),
        &dir,
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("core rules must load:\n{e}"));
    Engine::new(vec![set], settings()).unwrap()
}

/// Core rules whose cases or examples fail today, with the reason. Rule data belongs to the
/// rule-accuracy epic (slopvac-8l4.6); this list is the work it has to clear. A rule that
/// starts to pass must leave the list.
const KNOWN_FAILURES: &[(&str, &str)] = &[];

#[test]
fn every_core_case_and_example_passes_but_the_known_failures() {
    let report = run_cases(&core_engine(), None).unwrap();
    let known: Vec<&str> = KNOWN_FAILURES.iter().map(|(r, _)| *r).collect();
    let unknown = report.without(&known);
    assert!(unknown.is_clean(), "{unknown}");
    let listed: Vec<&str> = report.failing_rules().iter().map(|r| r.as_str()).collect();
    for rule in &known {
        assert!(
            listed.contains(rule),
            "{rule} passes now: remove it from KNOWN_FAILURES"
        );
    }
    assert!(report.passed() > 0, "{report}");
}

fn pack_dir(want: u32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let (name, text) = common::category(
        "cat",
        &[foo_rule(
            &json!({"cases": [{"input": "foo", "want": want}]}),
        )],
    );
    std::fs::write(dir.path().join(name), text).unwrap();
    dir
}

fn cli(args: &[&str]) -> (i32, String) {
    let m = slopvac_engine::cases::cli::command()
        .try_get_matches_from(std::iter::once("cases").chain(args.iter().copied()))
        .unwrap();
    let mut out = Vec::new();
    let code = slopvac_engine::cases::cli::run(&m, &mut out).unwrap();
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn the_command_exits_0_when_every_case_passes() {
    let dir = pack_dir(1);
    let (code, out) = cli(&[dir.path().to_str().unwrap()]);
    assert_eq!((code, out.as_str()), (0, "1 passed, 0 failed, 0 skipped\n"));
}

#[test]
fn the_command_exits_1_on_a_failed_case_and_prints_the_coverage_of_a_corpus() {
    let dir = pack_dir(2);
    let doc = dir.path().join("doc.md");
    std::fs::write(&doc, "no match here").unwrap();
    let (code, out) = cli(&[
        dir.path().to_str().unwrap(),
        "--coverage",
        doc.to_str().unwrap(),
    ]);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "core:cat.r cases[0]: want 2 finding(s), got 1\n0 passed, 1 failed, 0 skipped\n\
         core:cat.r: matched nothing\n0 of 1 rules matched in 1 document(s); 1 matched nothing\n"
    );
}

/// The committed eval documents: every `*.corpus` file under `evals/corpus`.
fn eval_corpus() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "corpus") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/corpus"),
        &mut files,
    );
    let mut docs: Vec<_> = files
        .iter()
        .map(|p| (p.display().to_string(), std::fs::read_to_string(p).unwrap()))
        .collect();
    docs.sort();
    docs
}

#[test]
fn coverage_report_over_the_eval_corpus() {
    let docs = eval_corpus();
    assert!(docs.len() > 100, "the committed eval corpus is missing");
    let inputs: Vec<Input<'_>> = docs
        .iter()
        .map(|(path, text)| Input::markdown(path, text))
        .collect();
    let cov = coverage(&core_engine(), &inputs).unwrap();
    // CI reads the report from the test log and from this file.
    let report = cov.to_string();
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("core-rule-coverage.txt");
    std::fs::write(&out, format!("{report}\n")).unwrap();
    eprintln!("{report}\nwritten to {}", out.display());
    assert_eq!(cov.documents, docs.len());
    // 17 tokens, 18 substitution and 109 pattern rules are mechanical.
    assert_eq!(cov.findings.len(), 144);
    assert!(
        cov.findings.values().any(|n| *n > 0),
        "no core rule matched anything in the corpus"
    );
}
