//! `Blocklist::merge`: the first list wins for a word and part of speech it shares with a
//! later one, and distinct entries from both survive.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use slopvac_terms::{Blocklist, Pos};

fn list(text: &str) -> Blocklist {
    Blocklist::parse("list.toml", "toml", text).expect("a valid blocklist")
}

const FIRST: &str = r#"schema_version = 3

[[entries]]
word = "leverage"
pos = "verb"
reason = "first reason"
replacement = "use"

[[entries]]
word = "utilize"
pos = "verb"
reason = "only in the first list"
"#;

const SECOND: &str = r#"schema_version = 3

[[entries]]
word = "leverage"
pos = "verb"
reason = "second reason"
replacement = "employ"

[[entries]]
word = "synergy"
pos = "noun"
reason = "only in the second list"
"#;

#[test]
fn distinct_entries_of_both_lists_survive() {
    let mut merged = list(FIRST);
    merged.merge(list(SECOND));
    let words: Vec<&str> = merged.iter().map(|e| e.word.as_str()).collect();
    assert_eq!(words, ["leverage", "synergy", "utilize"]);
    assert_eq!(merged.len(), 3);
}

#[test]
fn a_repeated_word_and_part_of_speech_keeps_the_first_files_reason_and_replacement() {
    let mut merged = list(FIRST);
    merged.merge(list(SECOND));
    let entry = merged.lookup("leverage", Pos::Verb).expect("listed");
    assert_eq!(entry.reason, "first reason");
    assert_eq!(entry.replacement.as_deref(), Some("use"));
}

#[test]
fn the_order_of_the_merge_decides_which_file_wins() {
    let mut merged = list(SECOND);
    merged.merge(list(FIRST));
    let entry = merged.lookup("leverage", Pos::Verb).expect("listed");
    assert_eq!(entry.reason, "second reason");
    assert_eq!(entry.replacement.as_deref(), Some("employ"));
}
