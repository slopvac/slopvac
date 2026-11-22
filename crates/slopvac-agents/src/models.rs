//! Reviewer model recommendations per review type, generated from eval results.
//!
//! `python -m evals models recommend` turns an eval results file into
//! `data/model-evals.json`: one row per reviewer model and review type, with the
//! success rate over the eval tasks. This module turns that file into the dated
//! table in `docs/model-recommendations.md` and the "pick the first available"
//! lines in `slopvac prime review`. Both come from one file, so they cannot
//! disagree. A test fails when the checked-in table differs from the generated one.

use std::fmt::Write as _;

use serde::Deserialize;

use crate::error::AgentsError;
use crate::messages::{AgentsMessage, AgentsPhrase, AgentsText, quoted};
use crate::review::ReviewType;

/// The embedded eval results.
pub const EMBEDDED: &str = include_str!("../data/model-evals.json");

/// Version of the file format the eval harness writes.
const SCHEMA_VERSION: u8 = 2;

/// Fewest repeat runs a result needs to count.
pub const MIN_REPEATS: u32 = 3;

/// Reviewers listed per review type in prime.
const PRIME_LIMIT: usize = 5;

/// One eval result for a reviewer model on one review type.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Run {
    /// `mechanical`, `update` or `rewrite`.
    pub review_type: String,
    /// Model family, for example `anthropic`.
    pub family: String,
    /// Tier within the family, for example `frontier`.
    pub tier: String,
    /// Model name as a harness reports it.
    pub model: String,
    /// Share of reviewer tasks passed, from 0 to 1. Repeats of a task are averaged first.
    pub success_rate: f64,
    /// Distinct eval tasks behind the rate.
    pub tasks: u32,
    /// Runs behind the rate, repeats included.
    pub runs: u32,
}

/// Whether the file may be shown to users as recommendations.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// The results are being rescored. The file holds no rows and nothing is published.
    PendingReevaluation,
    /// The rows are published as recommendations.
    Published,
}

/// A results file.
#[derive(Debug, Clone, Deserialize)]
pub struct Evals {
    /// Format version.
    pub schema_version: u8,
    /// Publication status.
    pub status: Status,
    /// Date of the eval run, `YYYY-MM-DD`.
    pub date: String,
    /// Repeats of each task in the eval run.
    pub repeats: u32,
    /// Reviewer results.
    pub rows: Vec<Run>,
}

fn invalid(detail: impl Into<String>) -> AgentsError {
    AgentsError::coded(&AgentsMessage::ModelEvalsInvalid {
        detail: detail.into(),
    })
}

impl Evals {
    /// Parse and validate a results file.
    pub fn parse(json: &str) -> Result<Self, AgentsError> {
        let evals: Self = serde_json::from_str(json).map_err(|e| invalid(e.to_string()))?;
        if evals.schema_version != SCHEMA_VERSION {
            return Err(invalid(
                AgentsText::EvalsSchema {
                    version: evals.schema_version,
                }
                .to_string(),
            ));
        }
        if evals.status == Status::PendingReevaluation && !evals.rows.is_empty() {
            return Err(invalid(
                AgentsPhrase::EvalsPendingRows.as_static().to_owned(),
            ));
        }
        if evals.repeats < MIN_REPEATS {
            return Err(invalid(
                AgentsText::EvalsRepeats {
                    repeats: evals.repeats,
                    minimum: MIN_REPEATS,
                }
                .to_string(),
            ));
        }
        for r in &evals.rows {
            if ReviewType::parse(&r.review_type).is_none() {
                return Err(invalid(
                    AgentsText::EvalsReviewType {
                        review_type: quoted(&r.review_type),
                    }
                    .to_string(),
                ));
            }
            if !(0.0..=1.0).contains(&r.success_rate) {
                return Err(invalid(
                    AgentsText::EvalsSuccessRate {
                        model: r.model.clone(),
                    }
                    .to_string(),
                ));
            }
        }
        Ok(evals)
    }

    /// The embedded results.
    pub fn embedded() -> Result<Self, AgentsError> {
        Self::parse(EMBEDDED)
    }

    /// Results for one review type, best score first, ties by model name.
    #[must_use]
    pub fn ranked(&self, kind: ReviewType) -> Vec<&Run> {
        let mut runs: Vec<&Run> = self
            .rows
            .iter()
            .filter(|r| r.review_type == kind.name())
            .collect();
        runs.sort_by(|a, b| {
            b.success_rate
                .total_cmp(&a.success_rate)
                .then_with(|| a.model.cmp(&b.model))
        });
        runs
    }
}

// STUB(slopvac-8l4.11.10): the data file holds no rows; they come back after the reviewer results are rescored.
// STUB(slopvac-8l4.11.11): mechanical and update reviewer rows come from the 15.8 run
const TYPES: [ReviewType; 3] = [
    ReviewType::Mechanical,
    ReviewType::Update,
    ReviewType::Rewrite,
];

/// A sentence naming the review types that have results, when some have none.
fn coverage(evals: &Evals) -> Option<String> {
    let covered: Vec<&str> = TYPES
        .into_iter()
        .filter(|k| !evals.ranked(*k).is_empty())
        .map(ReviewType::name)
        .collect();
    (!covered.is_empty() && covered.len() < TYPES.len()).then(|| {
        AgentsText::ModelsCoverage {
            types: covered.join(" and "),
        }
        .to_string()
    })
}

/// The line that points readers to the guide on setting models.
const GUIDE_LINK: &str =
    "\nTo set the models, read [Choosing models for review](guides/choosing-review-models.md).\n";

/// The generated `docs/model-recommendations.md`.
#[must_use]
pub fn markdown(evals: &Evals) -> String {
    let mut out = String::from(
        "<!-- generated by slopvac-agents from data/model-evals.json; do not edit -->\n",
    );
    out.push_str("# Reviewer model recommendations\n\n");
    if evals.status == Status::PendingReevaluation {
        out.push_str(AgentsPhrase::ModelsPending.as_static());
        out.push('\n');
        out.push_str(GUIDE_LINK);
        return out;
    }
    // The recommendations page is generated content, not a message.
    // ast-grep-ignore: user-text-literal
    let _ = write!(out, "Eval results dated {}. ", evals.date);
    if let Some(sentence) = coverage(evals) {
        let _ = write!(out, "{sentence} ");
    }
    out.push_str("Pick the first available model in a table. Use a reviewer from a different family than the author's model.\n");
    out.push_str(GUIDE_LINK);
    for kind in TYPES {
        let _ = write!(out, "\n## {}\n\n", kind.name());
        let ranked = evals.ranked(kind);
        if ranked.is_empty() {
            out.push_str(AgentsPhrase::ModelsNoneForType.as_static());
            out.push('\n');
            continue;
        }
        out.push_str("| Rank | Model | Family | Tier | Success rate | Tasks | Runs |\n| --- | --- | --- | --- | --- | --- | --- |\n");
        for (i, r) in ranked.iter().enumerate() {
            let _ = writeln!(
                out,
                "| {} | `{}` | {} | {} | {:.2} | {} | {} |",
                i + 1,
                r.model,
                r.family,
                r.tier,
                r.success_rate,
                r.tasks,
                r.runs
            );
        }
    }
    out
}

/// The lines `slopvac prime review` prints about reviewer models. With a type,
/// only that type's models; without one, the best model of each type.
#[must_use]
pub fn prime_guidance(evals: &Evals, kind: Option<ReviewType>) -> String {
    if evals.status == Status::PendingReevaluation {
        return AgentsPhrase::ModelsPending.as_static().into();
    }
    let line = |kind: ReviewType| -> Option<String> {
        let names: Vec<String> = evals
            .ranked(kind)
            .iter()
            .take(PRIME_LIMIT)
            .map(|r| format!("`{}` ({}, {})", r.model, r.family, r.tier))
            .collect();
        (!names.is_empty()).then(|| format!("{}: {}", kind.name(), names.join(", ")))
    };
    let lines: Vec<String> = match kind {
        Some(k) => line(k).into_iter().collect(),
        None => TYPES.into_iter().filter_map(line).collect(),
    };
    if lines.is_empty() {
        return AgentsPhrase::ModelsNone.as_static().into();
    }
    let mut out = AgentsText::ModelsPrimeHeader {
        date: evals.date.clone(),
    }
    .to_string();
    out.push('\n');
    if let Some(sentence) = coverage(evals) {
        let other = AgentsText::ModelsPrimeOtherTypes { coverage: sentence };
        let _ = writeln!(out, "{other}");
    }
    for l in lines {
        let _ = writeln!(out, "- {l}");
    }
    out.trim_end().to_owned()
}
