//! The case runner on synthetic rules and on the shipped core rules.

#[expect(
    dead_code,
    reason = "each test binary uses the part of the shared helpers it needs"
)]
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

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
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
        "core:cat.r cases[0]: want 2 finding(s), got 1\n0 passed, 1 failed, 0 incomplete"
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
fn a_good_example_the_engine_flags_fails() {
    // Raw text "foo **bar**" does not match `foo bar`, so the loader accepts it as a good
    // example. The prose projection drops the emphasis marks and the rule fires.
    let rule = common::rule(
        "p",
        &json!({"kind": "pattern", "pattern": "foo bar"}),
        &json!({"examples": [{"bad": "foo bar", "good": "foo **bar**"}]}),
    );
    let report = run_cases(&engine(&[rule]), None).unwrap();
    assert_eq!(
        outcomes(&report),
        [
            (Origin::ExampleBad(0), true),
            (Origin::ExampleGood(0), false)
        ]
    );
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
    let report = run_cases(
        &engine(&[
            judgement,
            foo_rule(&json!({"cases": [{"input": "foo", "want": 1}]})),
        ]),
        None,
    )
    .unwrap();
    assert_eq!(report.results.len(), 1);
}

#[test]
fn only_the_findings_of_the_rule_under_test_count() {
    let report = run_cases(
        &engine(&[
            foo_rule(&json!({"cases": [{"input": "foo bar", "want": 1}]})),
            tokens("other", "bar", &json!({})),
        ]),
        Some(&id("core:cat.r")),
    )
    .unwrap();
    assert!(report.is_clean(), "{report}");
    assert_eq!(report.passed(), 1);
}

#[expect(
    clippy::unreachable,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "the enum is non_exhaustive or defined in another crate, so a wildcard arm is required"
)]
fn reasons(report: &CaseReport) -> Vec<(Origin, String)> {
    report
        .incomplete()
        .map(|r| match &r.outcome {
            Outcome::Incomplete(why) => (r.origin, why.clone()),
            _ => unreachable!(),
        })
        .collect()
}

#[test]
fn a_rule_that_is_switched_off_is_incomplete_not_passed() {
    let set = ruleset(vec![common::category(
        "cat",
        &[foo_rule(&json!({"cases": [{"input": "foo", "want": 1}]}))],
    )]);
    let mut s = settings();
    s.rule_levels.insert(id("core:cat.r"), Level::Off);
    let report = run_cases(&Engine::new(vec![set], s).unwrap(), None).unwrap();
    assert_eq!(report.passed(), 0);
    assert_eq!(
        reasons(&report),
        [(
            Origin::Case(0),
            "the rule is switched off or does not apply under the engine settings".to_owned()
        )]
    );
    assert_eq!(report.exit_code(), 2);
}

fn passed_flags(report: &CaseReport) -> Vec<bool> {
    report
        .results
        .iter()
        .map(|r| r.outcome == Outcome::Passed)
        .collect()
}

fn incomplete_count(report: &CaseReport) -> usize {
    report.incomplete().count()
}

fn metric_rule(metric: &str) -> Value {
    common::rule(
        "m",
        &json!({"kind": "metric", "metric": metric, "threshold": 3, "comparison": "gt"}),
        &json!({"cases": [
            {"input": "One two.", "want": 0},
            {"input": "One two three four five six.", "want": 1},
        ]}),
    )
}

#[test]
fn a_supported_metric_executes_and_an_unsupported_one_is_incomplete() {
    let ok = run_cases(&engine(&[metric_rule("sentence_words")]), None).unwrap();
    assert_eq!(passed_flags(&ok), [true, true], "{ok}");
    let bad = run_cases(&engine(&[metric_rule("not_a_metric")]), None).unwrap();
    assert_eq!((bad.passed(), incomplete_count(&bad)), (0, 2), "{bad}");
}

fn caps_rule(style: &str) -> Value {
    common::rule(
        "c",
        &json!({"kind": "capitalization", "style": style}),
        &json!({"scope": "heading", "cases": [
            {"input": "# Install the plugin", "want": 0},
            {"input": "# Install The Plugin", "want": 1},
        ]}),
    )
}

#[test]
fn the_sentence_style_executes_and_other_capitalization_styles_are_incomplete() {
    let ok = run_cases(&engine(&[caps_rule("sentence")]), None).unwrap();
    assert_eq!(passed_flags(&ok), [true, true], "{ok}");
    let title = run_cases(&engine(&[caps_rule("title")]), None).unwrap();
    assert_eq!(
        (title.passed(), incomplete_count(&title)),
        (0, 2),
        "{title}"
    );
}

fn vocabulary_rule() -> Value {
    common::rule(
        "word-outside-controlled-vocabulary",
        &json!({"kind": "vocabulary", "check": "word-outside-controlled-vocabulary"}),
        &json!({"cases": [
            {"input": "The deploy failed.", "want": 1},
            {"input": "We deploy the app.", "want": 0},
        ]}),
    )
}

#[test]
fn a_vocabulary_rule_needs_a_blocklist_and_isolation_keeps_it() {
    let without = run_cases(&engine(&[vocabulary_rule()]), None).unwrap();
    assert_eq!(
        (without.passed(), incomplete_count(&without)),
        (0, 2),
        "{without}"
    );
    let list = slopvac_terms::Blocklist::parse(
        "b.toml",
        "toml",
        "schema_version = 3\n[[entries]]\nword = \"deploy\"\npos = \"noun\"\nreason = \"r\"\n",
    )
    .unwrap();
    let with = engine(&[vocabulary_rule()]).with_blocklist(&list).unwrap();
    let report = run_cases(&with, None).unwrap();
    assert_eq!(passed_flags(&report), [true, true], "{report}");
}

#[test]
fn an_engine_without_checkable_rules_is_an_error_not_a_pass() {
    let judgement = common::rule(
        "j",
        &json!({"kind": "judgement", "question": "Is it vague?"}),
        &json!({"severity": "suggestion"}),
    );
    let err = run_cases(&engine(&[judgement]), None).unwrap_err();
    assert_eq!(
        err.to_string(),
        "no rule has cases or checkable examples to run"
    );
}

#[test]
fn only_the_last_definition_of_a_duplicated_id_runs() {
    // Two sets define `cat.r`; the engine runs the later one, so the earlier one's failing
    // case must not appear.
    let shadowed = ruleset(vec![common::category(
        "cat",
        &[foo_rule(&json!({"cases": [{"input": "foo", "want": 5}]}))],
    )]);
    let effective = ruleset(vec![common::category(
        "cat",
        &[foo_rule(&json!({"cases": [{"input": "foo", "want": 1}]}))],
    )]);
    let e = Engine::new(vec![shadowed, effective], settings()).unwrap();
    let report = run_cases(&e, None).unwrap();
    assert_eq!(outcomes(&report), [(Origin::Case(0), true)]);
    let cov = coverage(&e, &[Input::markdown("a.md", "foo")]).unwrap();
    assert_eq!(cov.findings.len(), 1);
}

#[test]
fn coverage_leaves_out_an_id_that_a_later_non_lexical_definition_replaces() {
    let lexical = ruleset(vec![common::category("cat", &[foo_rule(&json!({}))])]);
    let judgement = ruleset(vec![common::category(
        "cat",
        &[common::rule(
            "r",
            &json!({"kind": "judgement", "question": "Is it vague?"}),
            &json!({"severity": "suggestion"}),
        )],
    )]);
    let e = Engine::new(vec![lexical, judgement], settings()).unwrap();
    let cov = coverage(&e, &[Input::markdown("a.md", "foo")]).unwrap();
    assert_eq!(cov.findings.len(), 0);
}

#[test]
fn a_rule_the_engine_cannot_execute_never_passes_a_zero_count_case() {
    let structure = common::rule(
        "s",
        &json!({"kind": "structure", "check": "heading-hierarchy"}),
        &json!({"cases": [{"input": "nothing to see", "want": 0}]}),
    );
    let report = run_cases(&engine(&[structure]), None).unwrap();
    assert_eq!(report.passed(), 0);
    assert_eq!(
        reasons(&report),
        [(
            Origin::Case(0),
            "the engine does not run this kind, style or metric of rule".to_owned()
        )]
    );
}

#[test]
fn a_testable_rule_without_cases_or_examples_is_incomplete() {
    let report = run_cases(&engine(&[tokens("bare", "foo", &json!({}))]), None).unwrap();
    assert_eq!(
        reasons(&report),
        [(
            Origin::Missing,
            "the rule has no cases or checkable examples".to_owned()
        )]
    );
    assert_eq!(report.exit_code(), 2);
}

#[test]
fn a_selected_judgement_rule_is_incomplete() {
    let judgement = common::rule(
        "j",
        &json!({"kind": "judgement", "question": "Is it vague?"}),
        &json!({"severity": "suggestion"}),
    );
    let report = run_cases(&engine(&[judgement]), Some(&id("core:cat.j"))).unwrap();
    assert_eq!(report.exit_code(), 2);
}

#[test]
fn an_unknown_selected_rule_is_an_error() {
    let e = engine(&[foo_rule(&json!({}))]);
    let err = run_cases(&e, Some(&id("core:cat.nope"))).unwrap_err();
    assert_eq!(err.to_string(), "no rule core:cat.nope");
}

#[test]
fn a_failed_case_outranks_an_incomplete_one_in_the_exit_code() {
    let report = run_cases(
        &engine(&[
            foo_rule(&json!({"cases": [{"input": "foo", "want": 2}]})),
            tokens("bare", "bar", &json!({})),
        ]),
        None,
    )
    .unwrap();
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn a_rule_that_exhausts_the_match_limit_cannot_abort_another_rules_cases() {
    // `^(a|aa)+$` on 64 a's and a b exhausts the PCRE2 match limit (slopvac-text pattern.rs).
    let bomb = common::rule(
        "bomb",
        &json!({"kind": "pattern", "pattern": "^(a|aa)+$"}),
        &json!({"cases": [{"input": format!("{}b", "a".repeat(64)), "want": 0}]}),
    );
    let ok = foo_rule(&json!({"cases": [
        {"input": format!("{}b", "a".repeat(64)), "want": 0},
        {"input": "foo", "want": 1},
    ]}));
    let report = run_cases(&engine(&[bomb, ok]), None).unwrap();
    let by_rule = |rule: &str| {
        report
            .results
            .iter()
            .filter(|r| r.rule == id(rule))
            .map(|r| matches!(r.outcome, Outcome::Passed))
            .collect::<Vec<_>>()
    };
    assert_eq!(by_rule("core:cat.r"), [true, true]);
    assert_eq!(report.failing_rules(), [&id("core:cat.bomb")], "{report}");
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

#[expect(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
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

const METRIC: &str = "metric rule: no cases, and examples do not run for non-lexical rules";
const STRUCTURE: &str = "structure rule: no cases, and the engine does not run structure checks";
const VOCABULARY: &str = "vocabulary rule: no cases, and it needs a word blocklist to run";

/// Core rules with no executable check, with the reason. They report `Incomplete`, so the
/// suite lists them instead of passing them. Authoring cases (and for structure and
/// vocabulary rules, engine support) clears an entry; the test fails if an entry is stale.
const NO_EXECUTABLE_CHECK: &[(&str, &str)] = &[
    (
        "core:ai-tells-content-shape.adjective-per-noun-spray",
        METRIC,
    ),
    ("core:ai-tells-formatting.em-dash-density", METRIC),
    ("core:ai-tells-formatting.bold-spray", METRIC),
    ("core:ai-tells-formatting.inline-header-list", METRIC),
    ("core:ai-tells-formatting.heading-hierarchy", STRUCTURE),
    ("core:prose-craft.sentence-length", METRIC),
    ("core:prose-discipline.run-on", METRIC),
    ("core:prose-format.prose-block", METRIC),
    ("core:ste-descriptive.sentence-too-long-descriptive", METRIC),
    ("core:ste-descriptive.paragraph-too-many-sentences", METRIC),
    ("core:ste-nouns.multiword-noun-too-long", METRIC),
    (
        "core:ste-practices.inconsistent-wording-for-same-step",
        STRUCTURE,
    ),
    ("core:ste-procedural.sentence-too-long-procedural", METRIC),
    (
        "core:ste-punctuation.colon-terminates-sentence-for-count",
        METRIC,
    ),
    (
        "core:ste-safety.safety-block-missing-consequence",
        STRUCTURE,
    ),
    (
        "core:ste-sentences.complex-text-not-in-vertical-list",
        METRIC,
    ),
    (
        "core:ste-sentences.vertical-list-lead-in-missing-colon",
        STRUCTURE,
    ),
    (
        "core:ste-sentences.vertical-list-item-punctuation",
        STRUCTURE,
    ),
    ("core:ste-verbs.verb-form-not-listed", VOCABULARY),
    (
        "core:ste-words.word-outside-controlled-vocabulary",
        VOCABULARY,
    ),
    (
        "core:ste-words.word-used-in-wrong-part-of-speech",
        VOCABULARY,
    ),
    (
        "core:ste-words.verb-or-adjective-form-not-permitted",
        VOCABULARY,
    ),
    ("core:ste-words.inconsistent-term-for-same-thing", STRUCTURE),
];

fn names(list: &[(&str, &str)]) -> Vec<String> {
    let mut names: Vec<String> = list.iter().map(|(r, _)| (*r).to_owned()).collect();
    names.sort();
    names
}

fn strings(rules: &[&slopvac_model::RuleId]) -> Vec<String> {
    rules.iter().map(ToString::to_string).collect()
}

#[test]
fn every_core_case_and_example_passes_but_the_known_failures() {
    let report = run_cases(&core_engine(), None).unwrap();
    assert_eq!(
        strings(&report.failing_rules()),
        names(KNOWN_FAILURES),
        "{report}"
    );
    assert!(report.passed() > 500, "{report}");
}

#[test]
fn the_core_rules_without_an_executable_check_are_listed() {
    let report = run_cases(&core_engine(), None).unwrap();
    assert_eq!(
        strings(&report.incomplete_rules()),
        names(NO_EXECUTABLE_CHECK),
        "{report}"
    );
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
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

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
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
    assert_eq!(
        (code, out.as_str()),
        (0, "1 passed, 0 failed, 0 incomplete\n")
    );
}

#[test]
fn the_command_exits_2_on_an_incomplete_check() {
    let dir = tempfile::tempdir().unwrap();
    let (name, text) = common::category("cat", &[tokens("bare", "foo", &json!({}))]);
    std::fs::write(dir.path().join(name), text).unwrap();
    let (code, out) = cli(&[dir.path().to_str().unwrap()]);
    assert_eq!(code, 2);
    assert_eq!(
        out,
        "core:cat.bare cases: incomplete: the rule has no cases or checkable examples\n\
         0 passed, 0 failed, 1 incomplete\n"
    );
}

#[test]
fn the_command_rejects_a_pack_with_nothing_to_run() {
    let dir = tempfile::tempdir().unwrap();
    let judgement = common::rule(
        "j",
        &json!({"kind": "judgement", "question": "Is it vague?"}),
        &json!({"severity": "suggestion"}),
    );
    let (name, text) = common::category("cat", &[judgement]);
    std::fs::write(dir.path().join(name), text).unwrap();
    let m = slopvac_engine::cases::cli::command()
        .try_get_matches_from(["cases", dir.path().to_str().unwrap()])
        .unwrap();
    let err = slopvac_engine::cases::cli::run(&m, &mut Vec::new()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "no rule has cases or checkable examples to run"
    );
}

#[test]
fn the_command_rejects_an_unknown_rule() {
    let dir = pack_dir(1);
    let m = slopvac_engine::cases::cli::command()
        .try_get_matches_from([
            "cases",
            dir.path().to_str().unwrap(),
            "--rule",
            "core:cat.nope",
        ])
        .unwrap();
    let err = slopvac_engine::cases::cli::run(&m, &mut Vec::new()).unwrap_err();
    assert_eq!(err.to_string(), "no rule core:cat.nope");
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
        "core:cat.r cases[0]: want 2 finding(s), got 1\n0 passed, 1 failed, 0 incomplete\n\
         core:cat.r: matched nothing\n0 of 1 rules matched in 1 document(s); 1 matched nothing\n"
    );
}

/// The committed eval documents: every `*.corpus` file under `evals/corpus`.
#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn eval_corpus() -> Vec<(String, String)> {
    #[expect(
        clippy::unwrap_used,
        reason = "test helper outside a test fn: a setup failure should fail the calling test"
    )]
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

/// Lint `docs` with the core rules and write the coverage report to `file` in the target
/// directory, so CI and a reader find it next to the test log.
#[expect(
    clippy::print_stderr,
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn core_coverage_report(docs: &[&(String, String)], file: &str) {
    let inputs: Vec<Input<'_>> = docs
        .iter()
        .map(|(path, text)| Input::markdown(path, text))
        .collect();
    let cov = coverage(&core_engine(), &inputs).unwrap();
    let report = cov.to_string();
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(file);
    std::fs::write(&out, format!("{report}\n")).unwrap();
    eprintln!("{report}\nwritten to {}", out.display());
    assert_eq!(cov.documents, docs.len());
    // The lexical rules: tokens, substitution and pattern.
    assert_eq!(cov.findings.len(), 146);
    assert!(
        cov.findings.values().any(|n| *n > 0),
        "no core rule matched anything in the corpus"
    );
}

/// Every 12th committed eval document by path: about 40 documents, which a debug build
/// lints in seconds. Rules without a match are reported, not failures.
#[test]
fn coverage_report_over_a_sample_of_the_eval_corpus() {
    let docs = eval_corpus();
    assert!(docs.len() > 100, "the committed eval corpus is missing");
    let sample: Vec<_> = docs.iter().step_by(12).collect();
    core_coverage_report(&sample, "core-rule-coverage-sample.txt");
}

/// The full report over all committed eval documents. It takes minutes in a debug build:
/// `cargo test -p slopvac-engine --test cases -- --ignored coverage_report`.
#[test]
#[ignore = "lints the whole eval corpus; run it for the full coverage report"]
fn coverage_report_over_the_eval_corpus() {
    let docs = eval_corpus();
    assert!(docs.len() > 100, "the committed eval corpus is missing");
    core_coverage_report(&docs.iter().collect::<Vec<_>>(), "core-rule-coverage.txt");
}
