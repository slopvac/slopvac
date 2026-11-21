//! Benchmark, not a correctness test: core rule patterns over the eval corpus on the PCRE2
//! interpreter (the default) and on the JIT (opt-in). Run it with
//!
//! ```sh
//! cargo test --release -p slopvac-text --test jit_bench -- --ignored --nocapture
//! ```
//!
//! It prints the totals and the ratio; it asserts only that both engines find the same
//! matches, so a drift between them still fails.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(
    clippy::allow_attributes_without_reason,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]
#![allow(
    missing_docs,
    clippy::cast_precision_loss,
    clippy::unchecked_time_subtraction
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;
use slopvac_text::Pattern;

fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// `(pattern, ignore_case)` of every `pattern`-kind rule in the core rule fixtures.
fn core_patterns(root: &Path) -> Vec<(String, bool)> {
    let mut paths = Vec::new();
    files(
        &root.join("../slopvac-rules/tests/fixtures/core-v3"),
        "json",
        &mut paths,
    );
    let mut out = Vec::new();
    for path in paths {
        let doc: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        for rule in doc["rules"].as_array().into_iter().flatten() {
            let m = &rule["match"];
            if m["kind"] == "pattern" {
                out.push((
                    m["pattern"].as_str().unwrap().to_owned(),
                    m["ignore_case"].as_bool().unwrap_or(false),
                ));
            }
        }
    }
    out
}

#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn interpreter_against_jit_on_core_rules_over_the_eval_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let patterns = core_patterns(root);
    let mut corpus_files = Vec::new();
    files(
        &root.join("../../evals/corpus"),
        "corpus",
        &mut corpus_files,
    );
    let texts: Vec<String> = corpus_files
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    let bytes: usize = texts.iter().map(String::len).sum();

    let mut totals = [Duration::ZERO; 2];
    let mut limit_hits = [0_usize; 2];
    let mut compiled = 0;
    let mut matches = [0_usize; 2];
    let mut per_pattern: Vec<(Duration, Duration, String)> = Vec::new();
    for (source, caseless) in &patterns {
        let (Ok(plain), Ok(fast)) = (
            Pattern::with_jit(source, *caseless, false),
            Pattern::with_jit(source, *caseless, true),
        ) else {
            continue;
        };
        compiled += 1;
        let before = totals;
        for (mode, pattern) in [&plain, &fast].into_iter().enumerate() {
            let start = Instant::now();
            for text in &texts {
                match pattern.find_all(text) {
                    Ok(found) => matches[mode] += found.len(),
                    Err(_) => limit_hits[mode] += 1,
                }
            }
            totals[mode] += start.elapsed();
        }
        per_pattern.push((
            totals[0] - before[0],
            totals[1] - before[1],
            source.chars().take(70).collect(),
        ));
    }
    per_pattern.sort_by_key(|row| std::cmp::Reverse(row.0));
    for (slow, quick, source) in per_pattern.iter().take(5) {
        println!("  {slow:>10.2?} interp {quick:>10.2?} jit  {source}");
    }
    println!(
        "{compiled} of {} core patterns, {} files, {:.1} MiB",
        patterns.len(),
        texts.len(),
        bytes as f64 / (1024.0 * 1024.0)
    );
    println!(
        "interpreter {:?} ({} matches, {} limit hits)",
        totals[0], matches[0], limit_hits[0]
    );
    println!(
        "jit         {:?} ({} matches, {} limit hits)",
        totals[1], matches[1], limit_hits[1]
    );
    println!(
        "interpreter / jit = {:.2}x",
        totals[0].as_secs_f64() / totals[1].as_secs_f64()
    );
    assert_eq!(matches[0], matches[1], "the engines disagree");
}
