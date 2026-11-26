//! Roll several documents into one run summary. A port of `slopvac.report.summarize`.

use std::collections::BTreeMap;

use slopvac_model::{
    AiRegister, AiRegisterSummary, CategoryName, CategorySummary, Density, Document, RunSummary,
    Score,
};

/// Round to `places` decimals the way Python's `round` does: half to even, judged on the exact
/// value of the float. The exact decimal expansion that Rust's formatter rounds is that value,
/// so a tie such as 96.25 goes to 96.2 and 2.675, which sits just below its tie, to 2.67.
fn round(value: f64, places: usize) -> f64 {
    format!("{value:.places$}").parse().unwrap_or(value)
}

fn density(value: f64) -> Density {
    Density::new(value.max(0.0)).unwrap_or(Density::ZERO)
}

fn per_100(findings: usize, words: usize) -> Density {
    if words == 0 {
        Density::ZERO
    } else {
        #[allow(clippy::cast_precision_loss, reason = "counts far below 2^52")]
        density(round(findings as f64 / words as f64 * 100.0, 3))
    }
}

fn axis<'a>(
    documents: &'a [Document],
    pick: impl Fn(&'a Document) -> &'a AiRegisterSummary,
) -> AiRegisterSummary {
    let words: usize = documents.iter().map(|d| d.words).sum();
    let values: Vec<&AiRegisterSummary> = documents.iter().map(pick).collect();
    let findings = values.iter().map(|v| v.findings).sum();
    AiRegisterSummary {
        findings,
        errors: values.iter().map(|v| v.errors).sum(),
        warnings: values.iter().map(|v| v.warnings).sum(),
        per_100_words: per_100(findings, words),
    }
}

/// The summary of `documents`. No documents give a clean, passing summary.
#[must_use]
pub fn summarize(documents: &[Document]) -> RunSummary {
    let words: usize = documents.iter().map(|d| d.words).sum();
    let findings: usize = documents.iter().map(|d| d.total_findings).sum();
    let overall = if documents.is_empty() {
        100.0
    } else {
        let weight = |d: &Document| d.words.max(1);
        #[allow(clippy::cast_precision_loss, reason = "word counts far below 2^52")]
        let denominator: f64 = documents.iter().map(|d| weight(d) as f64).sum();
        #[allow(clippy::cast_precision_loss)]
        if words > 0 {
            documents
                .iter()
                .map(|d| d.score.get() * weight(d) as f64)
                .sum::<f64>()
                / denominator
        } else {
            documents.iter().map(|d| d.score.get()).sum::<f64>() / documents.len() as f64
        }
    };

    let mut buckets: BTreeMap<&CategoryName, Vec<&slopvac_model::CategoryScore>> = BTreeMap::new();
    for document in documents {
        for entry in &document.categories {
            buckets.entry(&entry.category).or_default().push(entry);
        }
    }
    let categories = buckets
        .into_iter()
        .map(|(category, entries)| {
            let found: usize = entries.iter().map(|e| e.findings).sum();
            #[allow(clippy::cast_precision_loss)]
            let mean = entries.iter().map(|e| e.score.get()).sum::<f64>() / entries.len() as f64;
            CategorySummary {
                category: category.clone(),
                findings: found,
                errors: entries.iter().map(|e| e.errors).sum(),
                warnings: entries.iter().map(|e| e.warnings).sum(),
                suggestions: entries.iter().map(|e| e.suggestions).sum(),
                per_100_words: per_100(found, words),
                score: Score::new(round(mean, 1).clamp(0.0, 100.0)).unwrap_or(Score::BEST),
            }
        })
        .collect();

    RunSummary {
        documents: documents.len(),
        words,
        findings,
        errors: documents.iter().map(|d| d.errors).sum(),
        warnings: documents.iter().map(|d| d.warnings).sum(),
        suggestions: documents.iter().map(|d| d.suggestions).sum(),
        per_100_words: per_100(findings, words),
        score: Score::new(round(overall, 1).clamp(0.0, 100.0)).unwrap_or(Score::BEST),
        passed: documents.iter().all(|d| d.gate.is_passed()),
        categories,
        ai_register: AiRegister {
            strong: axis(documents, |d| &d.ai_register.strong),
            weak: axis(documents, |d| &d.ai_register.weak),
        },
        prose: axis(documents, |d| &d.prose),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_documents_summarize_as_clean() {
        let s = summarize(&[]);
        assert_eq!((s.documents, s.words, s.findings), (0, 0, 0));
        assert!((s.score.get() - 100.0).abs() < f64::EPSILON);
        assert!(s.passed);
    }

    #[test]
    fn rounding_matches_python_round_half_even_on_the_float() {
        // Expected values from Python 3: round(96.25, 1) == 96.2, round(0.125, 2) == 0.12,
        // round(2.675, 2) == 2.67, round(1.115, 2) == 1.11, round(92.5, 0) == 92.0.
        for (value, places, expected) in [
            (96.25, 1, 96.2),
            (0.125, 2, 0.12),
            (2.675, 2, 2.67),
            (1.115, 2, 1.11),
            (92.5, 0, 92.0),
            (93.5, 0, 94.0),
        ] {
            assert!(
                (round(value, places) - expected).abs() < 1e-12,
                "{value} to {places} places"
            );
        }
    }

    #[test]
    fn per_100_rounds_to_three_places() {
        // 1 finding in 3 words is 33.333... per 100 words.
        assert!((per_100(1, 3).get() - 33.333).abs() < 1e-9);
        assert!(per_100(5, 0).get().abs() < f64::EPSILON);
    }
}
