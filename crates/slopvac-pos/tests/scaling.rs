//! Wall-clock benchmark: doubling an adversarial input should about double the
//! time to tag it. Timing is noisy under load, so this is opt-in:
//!
//! ```sh
//! cargo test --release -p slopvac-pos --test scaling -- --ignored --nocapture
//! ```
//!
//! The deterministic linearity gate is the work-counter test in
//! `src/tokenize.rs`.

use std::time::{Duration, Instant};

use slopvac_pos::Tagger;

#[path = "common/adversarial.rs"]
mod adversarial;

const SMALL: usize = 20_000;
const BIG: usize = 40_000;

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

#[test]
#[ignore = "wall-clock benchmark; run with --ignored"]
fn tagging_time_grows_linearly_on_adversarial_inputs() {
    for name in adversarial::NAMES {
        let small = best_of_three(&adversarial::adversarial(name, SMALL));
        let big = best_of_three(&adversarial::adversarial(name, BIG));
        let ratio = big.as_secs_f64() / small.as_secs_f64().max(1e-6);
        eprintln!("{name}: {small:?} -> {big:?} (x{ratio:.2})");
    }
}
