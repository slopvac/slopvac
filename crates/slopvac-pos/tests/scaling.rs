//! End-to-end scaling: doubling an adversarial input must not much more than
//! double the time to tag it. A quadratic pass shows up as a ratio near 4.

use std::time::{Duration, Instant};

use slopvac_pos::Tagger;

const SMALL: usize = 20_000;
const BIG: usize = 40_000;
/// Linear work doubles; allow noise. Quadratic work quadruples.
const MAX_RATIO: f64 = 3.2;

fn best_of_three(text: &str) -> Duration {
    let tagger = Tagger::english();
    (0..3)
        .map(|_| {
            let start = Instant::now();
            let tokens = tagger.tag_text(text);
            std::hint::black_box(&tokens);
            start.elapsed()
        })
        .min()
        .unwrap()
}

/// Repeats `unit` until the text holds `n` copies.
fn adversarial(name: &str, n: usize) -> String {
    match name {
        // One chunk of suffix punctuation: every step peels one character.
        "bang run" => "!".repeat(n),
        "prefix run" => "(".repeat(n),
        // Prefix and suffix runs around contraction-free text.
        "wrapped word" => format!("{}word{}", "(".repeat(n / 2), ")".repeat(n / 2)),
        // Contractions at every step, with and without suffixes after them.
        "contractions" => "it's".repeat(n / 4),
        "contraction tail" => format!("a'll{}", "!".repeat(n)),
        "hyphen chain" => "ab-".repeat(n / 3),
        "dotted abbreviation tail" => format!("{}{}", "a.".repeat(n / 2), "!".repeat(n / 2)),
        "long word" => "a".repeat(n),
        "many tiny sentences" => "Go. ".repeat(n / 4),
        "unclosed backticks" => "`a ".repeat(n / 3),
        "backtick runs" => (1..)
            .scan(0, |len, k| {
                *len += k;
                (*len < n).then(|| format!("{} ", "`".repeat(k)))
            })
            .collect(),
        "closed code spans" => "`ab` ".repeat(n / 5),
        "quote pairs" => "\u{201c}x\u{201d} ".repeat(n / 4),
        other => panic!("unknown input {other}"),
    }
}

#[test]
fn tagging_time_grows_linearly_on_adversarial_inputs() {
    let mut slow = Vec::new();
    for name in [
        "bang run",
        "prefix run",
        "wrapped word",
        "contractions",
        "contraction tail",
        "hyphen chain",
        "dotted abbreviation tail",
        "long word",
        "many tiny sentences",
        "unclosed backticks",
        "backtick runs",
        "closed code spans",
        "quote pairs",
    ] {
        let small = best_of_three(&adversarial(name, SMALL));
        let big = best_of_three(&adversarial(name, BIG));
        let ratio = big.as_secs_f64() / small.as_secs_f64().max(1e-6);
        eprintln!("{name}: {small:?} -> {big:?} (x{ratio:.2})");
        if ratio > MAX_RATIO {
            slow.push(format!("{name}: x{ratio:.2}"));
        }
    }
    assert!(slow.is_empty(), "superlinear growth: {slow:?}");
}
