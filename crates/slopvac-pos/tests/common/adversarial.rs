//! Adversarial inputs shared by the work-counter test (`src/tokenize.rs`) and
//! the timing benchmark (`tests/scaling.rs`).
#![allow(unreachable_pub, missing_docs, dead_code)]

/// Every adversarial input, by name.
pub const NAMES: [&str; 14] = [
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
    "long sentence",
];

/// An input of about `n` bytes shaped to stress one pass.
pub fn adversarial(name: &str, n: usize) -> String {
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
        // Every opener closes at the next run.
        "unclosed backticks" => "`a ".repeat(n / 3),
        // Runs of 1, 2, 3, ... backticks: each length occurs once, so about
        // sqrt(2n) openers never close. A scanner that searches to the end of
        // the text for each opener does Theta(n^1.5) work.
        "backtick runs" => (1..)
            .scan(0, |len, k| {
                *len += k;
                (*len < n).then(|| format!("{} ", "`".repeat(k)))
            })
            .collect(),
        "closed code spans" => "`ab` ".repeat(n / 5),
        "quote pairs" => "\u{201c}x\u{201d} ".repeat(n / 4),
        // Many words, no sentence end: one sentence is tagged as a whole.
        "long sentence" => "ab ".repeat(n / 3),
        other => panic!("unknown input {other}"),
    }
}
