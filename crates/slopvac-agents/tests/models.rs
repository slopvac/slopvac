//! Reviewer model recommendations: parsing, ranking, generated doc and prime text.

use std::path::PathBuf;

use slopvac_agents::models::{Evals, markdown, prime_guidance};
use slopvac_agents::prime::{self, Level, Settings};
use slopvac_agents::review::ReviewType;

fn run(kind: &str, model: &str, family: &str, tier: &str, rate: f64, runs: u32) -> String {
    format!(
        r#"{{"review_type":"{kind}","family":"{family}","tier":"{tier}","model":"{model}","success_rate":{rate},"tasks":4,"runs":{runs}}}"#
    )
}

fn file(version: u8, repeats: u32, rows: &[String]) -> String {
    format!(
        r#"{{"schema_version":{version},"status":"published","date":"2026-11-14","repeats":{repeats},"rows":[{}]}}"#,
        rows.join(",")
    )
}

fn pending() -> Evals {
    Evals::parse(
        r#"{"schema_version":2,"status":"pending-reevaluation","date":"2026-11-14","repeats":3,"rows":[]}"#,
    )
    .unwrap()
}

fn evals(rows: &[String]) -> Evals {
    Evals::parse(&file(2, 3, rows)).unwrap()
}

fn sample() -> Evals {
    evals(&[
        run("update", "model-b", "beta", "mid", 0.80, 12),
        run("update", "model-a", "alpha", "frontier", 0.95, 15),
        run("update", "model-c", "alpha", "small", 0.80, 12),
        run("rewrite", "model-a", "alpha", "frontier", 0.90, 12),
    ])
}

/// Only the rewrite reviewer has results, as in the checked-in data.
fn rewrite_only() -> Evals {
    evals(&[
        run("rewrite", "model-a", "alpha", "frontier", 0.90, 12),
        run("rewrite", "model-b", "beta", "fast", 0.60, 12),
    ])
}

#[test]
fn ranking_sorts_by_success_rate_then_model_name() {
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
    assert!(md.contains("| 1 | `model-a` | alpha | frontier | 0.95 | 4 | 15 |"));
    assert!(md.contains("| 3 | `model-c` | alpha | small | 0.80 | 4 | 12 |"));
    let mechanical = md
        .split("## mechanical")
        .nth(1)
        .unwrap()
        .split("## update")
        .next()
        .unwrap();
    assert!(mechanical.contains("No eval results for this review type."));
}

#[test]
fn partial_coverage_is_stated_in_the_table_and_in_prime() {
    let sentence = "Results exist only for the rewrite reviewer.";
    assert!(markdown(&rewrite_only()).contains(sentence));
    let prime = prime_guidance(&rewrite_only(), None);
    assert!(prime.contains(sentence));
    assert!(prime.contains("- rewrite: `model-a` (alpha, frontier), `model-b` (beta, fast)"));
    // Two of three types is still partial, and the sentence names both.
    assert!(
        markdown(&sample()).contains("Results exist only for the update and rewrite reviewer.")
    );
}

#[test]
fn full_coverage_adds_no_partial_coverage_sentence() {
    let full = evals(&[
        run("mechanical", "m", "f", "t", 0.9, 12),
        run("update", "m", "f", "t", 0.9, 12),
        run("rewrite", "m", "f", "t", 0.9, 12),
    ]);
    assert!(!markdown(&full).contains("Results exist only"));
    assert!(!prime_guidance(&full, None).contains("Results exist only"));
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
                12,
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
    assert!(!all.contains("- mechanical"));
}

#[test]
fn a_type_without_results_falls_back_to_the_family_rule() {
    assert_eq!(
        prime_guidance(&rewrite_only(), Some(ReviewType::Update)),
        "No eval results yet. Pick a model from a different family than the author's."
    );
    assert_eq!(
        prime_guidance(&evals(&[]), None),
        "No eval results yet. Pick a model from a different family than the author's."
    );
}

#[test]
fn invalid_results_are_rejected_with_a_code() {
    let cases = [
        file(1, 3, &[]),
        file(2, 2, &[]),
        file(2, 3, &[run("other", "m", "f", "t", 0.5, 12)]),
        file(2, 3, &[run("update", "m", "f", "t", 1.5, 12)]),
        r#"{"schema_version":2,"status":"published","date":"d","runs":[]}"#.to_owned(),
        r#"{"schema_version":2,"status":"soon","date":"d","repeats":3,"rows":[]}"#.to_owned(),
        format!(
            r#"{{"schema_version":2,"status":"pending-reevaluation","date":"d","repeats":3,"rows":[{}]}}"#,
            run("update", "m", "f", "t", 0.5, 12)
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
fn pending_data_publishes_no_ranking_in_the_table_or_in_prime() {
    let pending_text = "Reviewer model recommendations are pending re-evaluation.";
    let md = markdown(&pending());
    assert!(md.contains(pending_text));
    assert!(!md.contains("##"));
    assert!(!md.contains('|'));
    for kind in [None, Some(ReviewType::Rewrite)] {
        let prime = prime_guidance(&pending(), kind);
        assert!(prime.starts_with(pending_text), "{prime}");
        assert!(!prime.contains('`'));
    }
}

#[test]
fn embedded_data_is_pending_and_the_rendered_review_topic_says_so() {
    let e = Evals::embedded().unwrap();
    assert_eq!(e.rows.len(), 0);
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
    assert!(text.contains(&prime_guidance(&e, None)));
    assert!(text.contains("pending re-evaluation"));
    for banned in ["gpt-", "claude-", "slopvac-8l4", "STUB", "15.8"] {
        assert!(!text.contains(banned), "{banned}");
    }
}

/// Compare the table at `path` with `generated`. A set `SLOPVAC_BLESS` rewrites the file, unless
/// `CI` is also set: a CI run must never rewrite tracked files, so it fails instead.
/// `var` reads the environment, so tests pass their own values.
fn check_table(
    path: &std::path::Path,
    generated: &str,
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<(), String> {
    if var("SLOPVAC_BLESS").is_some() {
        if var("CI").is_some() {
            return Err("SLOPVAC_BLESS is not allowed when CI is set".to_owned());
        }
        std::fs::write(path, generated).map_err(|e| e.to_string())?;
    }
    let on_disk = std::fs::read_to_string(path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if on_disk == generated {
        Ok(())
    } else {
        Err("run `SLOPVAC_BLESS=1 cargo test -p slopvac-agents --test models`".to_owned())
    }
}

/// The checked-in table must equal the generated one. Set `SLOPVAC_BLESS=1` to rewrite it
/// outside CI.
#[test]
fn checked_in_table_matches_the_generated_one() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/model-recommendations.md");
    let generated = markdown(&Evals::embedded().unwrap());
    if let Err(message) = check_table(&path, &generated, |k| std::env::var_os(k)) {
        panic!("{message}");
    }
}

fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
    move |k| {
        pairs
            .iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| (*v).into())
    }
}

#[test]
fn bless_outside_ci_rewrites_the_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.md");
    std::fs::write(&path, "stale\n").unwrap();
    check_table(&path, "fresh\n", env_of(&[("SLOPVAC_BLESS", "1")])).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh\n");
}

#[rstest::rstest]
#[case::truthy("1")]
#[case::empty("")]
fn bless_in_ci_fails_and_leaves_the_table_unchanged(#[case] ci: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.md");
    std::fs::write(&path, "stale\n").unwrap();
    let err = check_table(
        &path,
        "fresh\n",
        env_of(&[("SLOPVAC_BLESS", "1"), ("CI", ci)]),
    )
    .unwrap_err();
    assert!(err.contains("CI"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "stale\n");
}
