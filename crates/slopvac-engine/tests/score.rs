//! Scores and gates, with expectations worked out by hand from the 2.x formulas.

#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

use std::collections::BTreeMap;

use slopvac_engine::{CategoryInfo, Thresholds, score_document};
use slopvac_model::{
    AiSignal, CategoryId, Contexts, Finding, LineCol, NonEmptyString, Notice, NoticeLevel, PackId,
    RuleId, Severity,
};

fn finding(category: &str, severity: Severity) -> Finding {
    Finding {
        rule_id: RuleId::new(format!("core:{category}.r")).unwrap(),
        pack: PackId::new("core").unwrap(),
        category: CategoryId::new(category).unwrap(),
        severity,
        message: NonEmptyString::new("m").unwrap(),
        start: LineCol::new(1, 1).unwrap(),
        extent: None,
        contexts: Contexts::default(),
        ai_signal: AiSignal::None,
        matched_text: String::new(),
        fix: None,
        provenance: None,
        origin: None,
    }
}

fn categories(budget: Option<f64>) -> BTreeMap<CategoryId, CategoryInfo> {
    BTreeMap::from([(
        CategoryId::new("a").unwrap(),
        CategoryInfo {
            weight: 1.0,
            budget,
        },
    )])
}

fn one_category(
    findings: &[Finding],
    words: usize,
    budget: Option<f64>,
    thresholds: &Thresholds,
) -> slopvac_engine::Scoring {
    score_document(findings, words, &categories(budget), thresholds, &[])
}

#[test]
fn a_clean_document_scores_100_and_passes() {
    let s = one_category(&[], 100, Some(2.0), &Thresholds::default());
    assert_eq!(s.score.get(), 100.0);
    assert!(s.gate.is_passed());
    assert_eq!(s.categories.len(), 1);
}

#[test]
fn a_category_inside_its_budget_scores_between_100_and_70() {
    // 1 error in 100 words is 1.0 weighted per 100. Budget 2.0: 100 - 30 * (1 / 2) = 85.
    let s = one_category(
        &[finding("a", Severity::Error)],
        100,
        Some(2.0),
        &Thresholds::default(),
    );
    assert_eq!(s.categories[0].score.get(), 85.0);
    assert_eq!(s.score.get(), 85.0);
    assert_eq!(s.categories[0].per_100_words.get(), 1.0);
    assert!(!s.categories[0].over_budget);
    assert!(s.gate.is_passed());
}

#[test]
fn a_category_over_budget_decays_to_zero_at_four_times_the_budget_and_fails_the_gate() {
    // 2 errors in 100 words, budget 1.0: 70 * (1 - (2 - 1) / 3) = 46.67, shown as 46.7.
    let s = one_category(
        &[finding("a", Severity::Error), finding("a", Severity::Error)],
        100,
        Some(1.0),
        &Thresholds::default(),
    );
    assert_eq!(s.score.get(), 46.7);
    assert!(s.categories[0].over_budget);
    assert_eq!(
        s.gate.reasons(),
        ["a at 2.00 severity-weighted findings per 100 words, budget 1.0"]
    );
}

#[test]
fn a_short_document_is_scored_on_counts_and_has_no_density() {
    // 50 words: one warning weighs 0.5, so 100 - 0.5 * 20 = 90.
    let s = one_category(
        &[finding("a", Severity::Warning)],
        50,
        Some(0.1),
        &Thresholds::default(),
    );
    assert_eq!(s.score.get(), 90.0);
    assert_eq!(s.per_100_words.get(), 0.0);
    assert!(!s.categories[0].over_budget);
}

#[test]
fn suggestions_cost_at_most_15_points_and_never_fail_the_gate() {
    // 6 suggestions in 100 words reach the full-penalty density of 6: 100 - 15 = 85.
    let many = vec![finding("a", Severity::Suggestion); 6];
    let s = one_category(&many, 100, Some(1.0), &Thresholds::default());
    assert_eq!(s.score.get(), 85.0);
    assert!(s.gate.is_passed());
    let more = vec![finding("a", Severity::Suggestion); 60];
    assert_eq!(
        one_category(&more, 100, Some(1.0), &Thresholds::default())
            .score
            .get(),
        85.0
    );
}

#[test]
fn a_category_with_no_budget_loses_ten_points_per_weighted_finding_per_100_words() {
    let s = one_category(
        &[finding("a", Severity::Error)],
        100,
        None,
        &Thresholds::default(),
    );
    assert_eq!(s.score.get(), 90.0);
}

#[test]
fn a_zero_weight_category_gates_nothing() {
    let mut cats = categories(Some(0.0));
    cats.get_mut(&CategoryId::new("a").unwrap()).unwrap().weight = 0.0;
    // With no weighted category left, every finding gates again, as in 2.x.
    cats.insert(
        CategoryId::new("b").unwrap(),
        CategoryInfo {
            weight: 1.0,
            budget: None,
        },
    );
    let s = score_document(
        &[finding("a", Severity::Error)],
        100,
        &cats,
        &Thresholds::default().with_max_errors(Some(0)),
        &[],
    );
    assert!(s.gate.is_passed());
    assert_eq!(s.errors, 1, "the raw counts still include it");
}

#[test]
fn thresholds_fail_the_gate_with_the_2x_reasons() {
    let findings = [
        finding("a", Severity::Error),
        finding("a", Severity::Warning),
        finding("a", Severity::Warning),
    ];
    let s = one_category(
        &findings,
        100,
        None,
        &Thresholds::default()
            .with_max_errors(Some(0))
            .with_max_warnings(Some(1))
            .with_max_total_per_100_words(Some(1.5))
            .with_min_score(Some(90.0)),
    );
    // Weighted density: 1 + 0.5 + 0.5 = 2.0 per 100. Without a category budget the score
    // is 100 - 2 * 10 = 80, and the document budget of 1.5 gives the whole-document score
    // 70 * (1 - (2 - 1.5) / 4.5) = 62.2; the lower one wins.
    assert_eq!(
        s.gate.reasons(),
        [
            "1 error(s), limit 0",
            "2 warning(s), limit 1",
            "2.00 severity-weighted findings per 100 words, budget 1.5",
            "score 62.2, minimum 90.0",
        ]
    );
}

#[test]
fn a_warning_notice_makes_the_check_incomplete_and_an_info_notice_does_not() {
    let notice = |level, message: &str| Notice {
        code: slopvac_model::KebabName::new("some-code").unwrap(),
        level,
        message: NonEmptyString::new(message).unwrap(),
        path: None,
        data: serde_json::Map::new(),
    };
    let unchecked = [
        notice(NoticeLevel::Warning, "metric did not run"),
        notice(NoticeLevel::Info, "fyi"),
    ];
    let s = score_document(
        &[],
        100,
        &categories(None),
        &Thresholds::default(),
        &unchecked,
    );
    assert_eq!(s.gate.reasons(), ["incomplete check: metric did not run"]);
    let info_only = score_document(
        &[],
        100,
        &categories(None),
        &Thresholds::default(),
        &unchecked[1..],
    );
    assert!(info_only.gate.is_passed());
}

#[test]
fn the_unicode_dash_limit_counts_that_rule_at_any_severity() {
    let mut dash = finding("prose-format", Severity::Suggestion);
    dash.rule_id = RuleId::new("core:prose-format.no-unicode-dash").unwrap();
    let s = one_category(
        &[dash.clone(), dash],
        100,
        None,
        &Thresholds::default().with_max_unicode_dashes(Some(1)),
    );
    assert_eq!(s.gate.reasons(), ["2 Unicode dash(es), limit 1"]);
}

#[test]
fn findings_are_counted_by_ai_signal() {
    let mut strong = finding("a", Severity::Error);
    strong.ai_signal = AiSignal::Strong;
    let plain = finding("a", Severity::Warning);
    let s = one_category(&[strong, plain], 100, None, &Thresholds::default());
    assert_eq!(s.ai_register.strong.findings, 1);
    assert_eq!(s.ai_register.strong.errors, 1);
    assert_eq!(s.ai_register.weak.findings, 0);
    assert_eq!(s.prose.findings, 1);
    assert_eq!(s.prose.warnings, 1);
    assert_eq!(s.prose.per_100_words.get(), 1.0);
}
