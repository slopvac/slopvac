//! Density, scores and the gate. A port of `slopvac.score`.
//!
//! Three numbers answer different questions. `per_100_words` is the raw density of every
//! finding. `gating_per_100_words` weights errors and warnings and is what a budget is
//! checked against. `score` runs from 0 to 100 and comes from the gating density against
//! the budget, less a bounded penalty for suggestions. A suggestion can lower a score but
//! never fails a gate on its own.

use std::collections::BTreeMap;

use slopvac_model::{
    AiRegister, AiRegisterSummary, AiSignal, CategoryName, CategoryScore, Density, Finding, Gate,
    Notice, NoticeLevel, Score, Severity,
};

use crate::settings::Thresholds;

/// Below this many words a density says nothing: one finding in a 20-word message reads as
/// 5 per 100 words. Such a document is scored on absolute counts.
pub(crate) const MIN_WORDS_FOR_DENSITY: usize = 60;
/// The density at which a category scores 0 is this multiple of its budget.
const ZERO_SCORE_MULTIPLE: f64 = 4.0;
/// The most a document loses to suggestions alone, in points.
const MAX_SUGGESTION_PENALTY: f64 = 15.0;
/// The suggestion density at which that penalty is spent.
const SUGGESTION_PENALTY_FULL_AT: f64 = 6.0;
/// The rule whose findings `Thresholds::max_unicode_dashes` counts.
const UNICODE_DASH_RULE: &str = "prose-format.no-unicode-dash";

/// A category as the scorer needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CategoryInfo {
    /// The weight in the document score. A zero weight gates nothing.
    pub weight: f64,
    /// The density budget, when the category has one.
    pub budget: Option<f64>,
}

/// Everything a document's score and gate say.
#[derive(Debug, Clone, PartialEq)]
pub struct Scoring {
    /// One entry per category, sorted by id.
    pub categories: Vec<CategoryScore>,
    /// Findings, all severities.
    pub total_findings: usize,
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Suggestions.
    pub suggestions: usize,
    /// Findings per 100 words.
    pub per_100_words: Density,
    /// The document score.
    pub score: Score,
    /// The verdict.
    pub gate: Gate,
    /// Findings by `ai_signal` strength.
    pub ai_register: AiRegister,
    /// Findings with no `ai_signal`.
    pub prose: AiRegisterSummary,
}

fn weight_of(severity: Severity) -> f64 {
    match severity {
        Severity::Error => 1.0,
        Severity::Warning => 0.5,
        Severity::Suggestion => 0.1,
    }
}

fn count(findings: &[&Finding], severity: Severity) -> usize {
    findings.iter().filter(|f| f.severity == severity).count()
}

/// Round to `places` decimals the way Python's `round` does: to the nearest decimal, ties
/// to even, on the exact binary value.
fn round(value: f64, places: usize) -> f64 {
    format!("{value:.places$}").parse().unwrap_or(value)
}

#[allow(clippy::cast_precision_loss)]
fn per_100(count: usize, words: usize) -> f64 {
    count as f64 / words as f64 * 100.0
}

fn measurable(words: usize) -> bool {
    words >= MIN_WORDS_FOR_DENSITY
}

fn density(value: f64) -> Density {
    Density::new(round(value, 3)).expect("a count over words is finite and not negative")
}

fn score(value: f64) -> Score {
    Score::new(round(value, 1).clamp(0.0, 100.0)).expect("a clamped score is valid")
}

fn suggestion_penalty(suggestion_density: f64) -> f64 {
    if suggestion_density <= 0.0 {
        return 0.0;
    }
    MAX_SUGGESTION_PENALTY * (suggestion_density / SUGGESTION_PENALTY_FULL_AT).min(1.0)
}

/// Map a weighted density onto 0 to 100. At or below the budget the score runs from 100
/// to 70. Above it the score falls from 70 to 0 at four times the budget.
fn score_from_density(weighted: f64, budget: Option<f64>) -> f64 {
    let Some(budget) = budget else {
        return (100.0 - weighted * 10.0).max(0.0);
    };
    if budget == 0.0 {
        return if weighted == 0.0 { 100.0 } else { 0.0 };
    }
    if weighted <= budget {
        return 100.0 - 30.0 * (weighted / budget);
    }
    let span = budget * (ZERO_SCORE_MULTIPLE - 1.0);
    if span <= 0.0 {
        return 0.0;
    }
    (70.0 * (1.0 - (weighted - budget) / span)).max(0.0)
}

fn score_from_counts(weighted: f64) -> f64 {
    (100.0 - weighted * 20.0).max(0.0)
}

fn gating_weight(findings: &[&Finding]) -> f64 {
    findings
        .iter()
        .filter(|f| f.severity != Severity::Suggestion)
        .map(|f| weight_of(f.severity))
        .sum()
}

/// A category's score and the weight it carries.
fn category_result(
    category: &CategoryName,
    items: &[&Finding],
    words: usize,
    info: CategoryInfo,
) -> (CategoryScore, f64) {
    let suggestions = count(items, Severity::Suggestion);
    let gating_weight = gating_weight(items);
    let measurable = measurable(words);
    let all = if measurable {
        per_100(items.len(), words)
    } else {
        0.0
    };
    let gating = if measurable {
        gating_weight / words_f(words) * 100.0
    } else {
        0.0
    };
    #[allow(clippy::cast_precision_loss)]
    let suggestion_density = if measurable {
        per_100(suggestions, words)
    } else {
        suggestions as f64
    };
    let base = if measurable {
        score_from_density(gating, info.budget)
    } else {
        score_from_counts(gating_weight)
    };
    let final_score = (base - suggestion_penalty(suggestion_density)).max(0.0);
    (
        CategoryScore {
            category: category.clone(),
            findings: items.len(),
            errors: count(items, Severity::Error),
            warnings: count(items, Severity::Warning),
            suggestions,
            per_100_words: density(all),
            gating_per_100_words: density(gating),
            budget: info
                .budget
                .map(|b| Density::new(b).expect("a budget is not negative")),
            score: score(final_score),
            over_budget: info.weight > 0.0 && info.budget.is_some_and(|b| gating > b),
        },
        info.weight,
    )
}

#[allow(clippy::cast_precision_loss)]
fn words_f(words: usize) -> f64 {
    words as f64
}

fn weighted_category_score(entries: &[(CategoryScore, f64)]) -> f64 {
    let active: Vec<(f64, f64)> = entries
        .iter()
        .filter(|(_, w)| *w > 0.0)
        .map(|(c, w)| (c.score.get(), *w))
        .collect();
    if active.is_empty() {
        return 100.0;
    }
    active.iter().map(|(s, w)| s * w).sum::<f64>() / active.iter().map(|(_, w)| w).sum::<f64>()
}

fn whole_document_score(relevant: &[&Finding], words: usize, budget: Option<f64>) -> f64 {
    let blocking = gating_weight(relevant);
    let suggestions = count(relevant, Severity::Suggestion);
    let measurable = measurable(words);
    let base = if measurable {
        score_from_density(blocking / words_f(words) * 100.0, budget)
    } else {
        score_from_counts(blocking)
    };
    #[allow(clippy::cast_precision_loss)]
    let suggestion_density = if measurable {
        per_100(suggestions, words)
    } else {
        suggestions as f64
    };
    (base - suggestion_penalty(suggestion_density)).max(0.0)
}

fn blocking_density(relevant: &[&Finding], words: usize) -> f64 {
    if !measurable(words) {
        return 0.0;
    }
    gating_weight(relevant) / words_f(words) * 100.0
}

#[allow(clippy::too_many_arguments)]
fn failure_reasons(
    findings: &[Finding],
    gated: &[&Finding],
    categories: &[CategoryScore],
    words: usize,
    overall: f64,
    thresholds: &Thresholds,
    unchecked: &[Notice],
) -> Vec<String> {
    let errors = count(gated, Severity::Error);
    let warnings = count(gated, Severity::Warning);
    let mut reasons = Vec::new();
    let incomplete: Vec<&str> = unchecked
        .iter()
        .filter(|n| n.level == NoticeLevel::Warning)
        .map(|n| n.message.as_str())
        .collect();
    if !incomplete.is_empty() {
        reasons.push(
            crate::messages::EngineText::GateIncomplete {
                reasons: incomplete.join("; "),
            }
            .to_string(),
        );
    }
    if let Some(limit) = thresholds.max_errors.filter(|l| errors > *l) {
        reasons.push(crate::messages::EngineText::GateErrors { errors, limit }.to_string());
    }
    if let Some(limit) = thresholds.max_warnings.filter(|l| warnings > *l) {
        reasons.push(crate::messages::EngineText::GateWarnings { warnings, limit }.to_string());
    }
    if let Some(limit) = thresholds.max_unicode_dashes {
        let dashes = findings
            .iter()
            .filter(|f| f.rule_id.local() == UNICODE_DASH_RULE)
            .count();
        if dashes > limit {
            reasons.push(crate::messages::EngineText::GateDashes { dashes, limit }.to_string());
        }
    }
    let blocking = blocking_density(gated, words);
    if let Some(budget) = thresholds.max_total_per_100_words
        && measurable(words)
        && blocking > budget
    {
        reasons.push(
            crate::messages::EngineText::GateDensity {
                density: format!("{blocking:.2}"),
                budget: py_float(budget),
            }
            .to_string(),
        );
    }
    if let Some(minimum) = thresholds.min_score
        && (errors > 0 || warnings > 0)
        && overall < minimum
    {
        reasons.push(
            crate::messages::EngineText::GateScore {
                score: format!("{overall:.1}"),
                minimum: py_float(minimum),
            }
            .to_string(),
        );
    }
    reasons.extend(categories.iter().filter(|c| c.over_budget).map(|c| {
        crate::messages::EngineText::GateCategory {
            category: c.category.to_string(),
            density: format!("{:.2}", c.gating_per_100_words.get()),
            budget: c
                .budget
                .map_or_else(|| "None".to_owned(), |b| py_float(b.get())),
        }
        .to_string()
    }));
    reasons
}

/// A float as Python prints it: `3.0` stays `3.0`, `2.5` stays `2.5`.
fn py_float(value: f64) -> String {
    let text = value.to_string();
    if text.contains(['.', 'e', 'n', 'i']) {
        text
    } else {
        format!("{text}.0")
    }
}

fn ai_summary(findings: &[Finding], words: usize, signal: AiSignal) -> AiRegisterSummary {
    let selected: Vec<&Finding> = findings.iter().filter(|f| f.ai_signal == signal).collect();
    let value = if measurable(words) {
        per_100(selected.len(), words)
    } else {
        0.0
    };
    AiRegisterSummary {
        findings: selected.len(),
        errors: count(&selected, Severity::Error),
        warnings: count(&selected, Severity::Warning),
        per_100_words: density(value),
    }
}

/// Score one document.
///
/// `categories` lists every category of the rule data, so a category with no finding still
/// appears. A finding whose category is absent from it counts with weight 1 and no budget.
#[must_use]
pub fn score_document(
    findings: &[Finding],
    words: usize,
    categories: &BTreeMap<CategoryName, CategoryInfo>,
    thresholds: &Thresholds,
    unchecked: &[Notice],
) -> Scoring {
    let mut by_category: BTreeMap<&CategoryName, Vec<&Finding>> =
        categories.keys().map(|k| (k, Vec::new())).collect();
    for finding in findings {
        by_category
            .entry(&finding.category)
            .or_default()
            .push(finding);
    }
    let entries: Vec<(CategoryScore, f64)> = by_category
        .iter()
        .map(|(name, items)| {
            let info = categories.get(*name).copied().unwrap_or(CategoryInfo {
                weight: 1.0,
                budget: None,
            });
            category_result(name, items, words, info)
        })
        .collect();
    let active: Vec<&CategoryName> = entries
        .iter()
        .filter(|(_, w)| *w > 0.0)
        .map(|(c, _)| &c.category)
        .collect();
    // With no weighted category every finding counts, as in the reference.
    let gated: Vec<&Finding> = if active.is_empty() {
        findings.iter().collect()
    } else {
        findings
            .iter()
            .filter(|f| active.contains(&&f.category))
            .collect()
    };
    let overall = weighted_category_score(&entries).min(whole_document_score(
        &gated,
        words,
        thresholds.max_total_per_100_words,
    ));
    let category_scores: Vec<CategoryScore> = entries.into_iter().map(|(c, _)| c).collect();
    let reasons = failure_reasons(
        findings,
        &gated,
        &category_scores,
        words,
        overall,
        thresholds,
        unchecked,
    );
    let all: Vec<&Finding> = findings.iter().collect();
    let per = if measurable(words) {
        per_100(findings.len(), words)
    } else {
        0.0
    };
    Scoring {
        categories: category_scores,
        total_findings: findings.len(),
        errors: count(&all, Severity::Error),
        warnings: count(&all, Severity::Warning),
        suggestions: count(&all, Severity::Suggestion),
        per_100_words: density(per),
        score: score(overall),
        gate: if reasons.is_empty() {
            Gate::passed()
        } else {
            Gate::failed(reasons).expect("reasons are not empty")
        },
        ai_register: AiRegister {
            strong: ai_summary(findings, words, AiSignal::Strong),
            weak: ai_summary(findings, words, AiSignal::Weak),
        },
        prose: ai_summary(findings, words, AiSignal::None),
    }
}
