//! Reviewer model recommendations: parsing, ranking, generated doc and prime text.
#![cfg(test)]

use std::path::PathBuf;

use slopvac_agents::models::{Evals, markdown, prime_guidance};
use slopvac_agents::prime::{self, Level, Settings};
use slopvac_agents::review::ReviewType;

fn run(kind: &str, model: &str, family: &str, tier: &str, score: f64, repeats: u32) -> String {
    format!(
        r#"{{"review_type":"{kind}","family":"{family}","tier":"{tier}","model":"{model}","score":{score},"repeats":{repeats}}}"#
    )
}

fn evals(runs: &[String]) -> Evals {
    Evals::parse(&format!(
        r#"{{"schema_version":1,"date":"2026-11-14","runs":[{}]}}"#,
        runs.join(",")
    ))
    .unwrap()
}

fn sample() -> Evals {
    evals(&[
        run("update", "model-b", "beta", "mid", 0.80, 3),
        run("update", "model-a", "alpha", "frontier", 0.95, 5),
        run("update", "model-c", "alpha", "small", 0.80, 3),
        run("rewrite", "model-a", "alpha", "frontier", 0.90, 4),
    ])
}

#[test]
fn ranking_sorts_by_score_then_model_name() {
    let s = sample();
    let names: Vec<&str> = s
        .ranked(ReviewType::Update)
        .iter()
        .map(|r| r.model.as_str())
        .collect();
    assert_eq!(names, ["model-a", "model-b", "model-c"]);
    assert_eq!(s.ranked(ReviewType::Mechanical).len(), 0);
}

#[test]
fn markdown_table_lists_each_type_with_ranks_and_empty_types_say_so() {
    let md = markdown(&sample());
    assert!(md.contains("Eval results dated 2026-11-14."));
    assert!(md.contains("| 1 | `model-a` | alpha | frontier | 0.95 | 5 |"));
    assert!(md.contains("| 3 | `model-c` | alpha | small | 0.80 | 3 |"));
    let mechanical = md
        .split("## mechanical")
        .nth(1)
        .unwrap()
        .split("## update")
        .next()
        .unwrap();
    assert!(mechanical.contains("No eval results yet."));
}

#[test]
fn prime_guidance_says_pick_the_first_available_and_limits_the_list() {
    let many: Vec<String> = (0..7)
        .map(|i| {
            run(
                "update",
                &format!("m{i}"),
                "f",
                "t",
                0.9 - f64::from(i) / 100.0,
                3,
            )
        })
        .collect();
    let text = prime_guidance(&evals(&many), Some(ReviewType::Update));
    assert!(
        text.starts_with("Reviewer models from evals dated 2026-11-14. Pick the first available:")
    );
    assert!(text.contains("`m4` (f, t)"));
    assert!(!text.contains("`m5`"));
    let all = prime_guidance(&sample(), None);
    assert!(all.contains("- update: `model-a` (alpha, frontier), `model-b`"));
    assert!(all.contains("- rewrite: `model-a`"));
    assert!(!all.contains("mechanical"));
}

#[test]
fn without_results_prime_falls_back_to_the_family_rule() {
    let empty = evals(&[]);
    assert_eq!(
        prime_guidance(&empty, None),
        "No eval results yet. Pick a model from a different family than the author's."
    );
}

#[test]
fn invalid_results_are_rejected_with_a_code() {
    let cases = [
        r#"{"schema_version":2,"date":"d","runs":[]}"#.to_owned(),
        format!(
            r#"{{"schema_version":1,"date":"d","runs":[{}]}}"#,
            run("other", "m", "f", "t", 0.5, 3)
        ),
        format!(
            r#"{{"schema_version":1,"date":"d","runs":[{}]}}"#,
            run("update", "m", "f", "t", 1.5, 3)
        ),
        format!(
            r#"{{"schema_version":1,"date":"d","runs":[{}]}}"#,
            run("update", "m", "f", "t", 0.5, 2)
        ),
        "not json".to_owned(),
    ];
    for c in cases {
        assert_eq!(
            Evals::parse(&c).unwrap_err().code(),
            "models-eval-invalid",
            "{c}"
        );
    }
}

#[test]
fn prime_review_lists_the_embedded_guidance() {
    let text = prime::render(
        Some("review"),
        &Settings {
            level: Level::Default,
            ..Settings::default()
        },
        &clap::Command::new("slopvac"),
        std::path::Path::new("/nonexistent"),
    )
    .unwrap();
    assert!(text.contains(&prime_guidance(&Evals::embedded().unwrap(), None)));
}

/// The checked-in table must equal the generated one. Set `SLOPVAC_BLESS=1` to rewrite it.
#[test]
fn checked_in_table_matches_the_generated_one() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/model-recommendations.md");
    let generated = markdown(&Evals::embedded().unwrap());
    if std::env::var_os("SLOPVAC_BLESS").is_some() {
        std::fs::write(&path, &generated).unwrap();
    }
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert_eq!(
        on_disk, generated,
        "run `SLOPVAC_BLESS=1 cargo test -p slopvac-agents --test models`"
    );
}
