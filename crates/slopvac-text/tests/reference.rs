//! Word counts, text types and sentences against the Python reference's recorded output.

use serde_json::Value;
use slopvac_text::{TextType, classify_text_type, count_words, split_sentences};

fn cases() -> Result<Vec<Value>, serde_json::Error> {
    serde_json::from_str(include_str!("fixtures/reference.json"))
}

fn kind(name: &str) -> Result<TextType, serde_json::Error> {
    serde_json::from_value(Value::String(name.to_owned()))
}

/// A JSON number as a count or offset.
fn index(value: &Value) -> usize {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(usize::MAX)
}

#[test]
fn word_counts_match_the_python_reference() {
    for c in cases().unwrap() {
        let text = c["text"].as_str().unwrap();
        assert_eq!(count_words(text).unwrap(), index(&c["words"]), "{text:?}");
    }
}

#[test]
fn text_types_match_the_python_reference() {
    for c in cases().unwrap() {
        let text = c["text"].as_str().unwrap();
        assert_eq!(
            classify_text_type(text).unwrap(),
            kind(c["text_type"].as_str().unwrap()).unwrap(),
            "{text:?}"
        );
    }
}

#[test]
fn sentences_match_the_python_reference() {
    for c in cases().unwrap() {
        let text = c["text"].as_str().unwrap();
        let got = split_sentences(text).unwrap();
        let want = c["sentences"].as_array().unwrap();
        assert_eq!(
            got.len(),
            want.len(),
            "sentence count for {text:?}: {got:?}"
        );
        for (g, w) in got.iter().zip(want) {
            assert_eq!(g.text, w["text"].as_str().unwrap(), "{text:?}");
            assert_eq!(g.start, index(&w["start"]), "{text:?}");
            assert_eq!(g.end, index(&w["end"]), "{text:?}");
            assert_eq!(g.word_count, index(&w["word_count"]), "{text:?}");
            assert_eq!(
                g.text_type,
                kind(w["text_type"].as_str().unwrap()).unwrap(),
                "{text:?}"
            );
            assert_eq!(text.get(g.start..g.end), Some(g.text.as_str()));
        }
    }
}

/// Intended divergence from the Python reference (tests/fixtures/divergences.md): the
/// lead-in of a vertical list reports its true start when the text begins with
/// whitespace. Python reports offset 0, which does not slice back to the sentence.
#[test]
fn list_lead_in_keeps_its_true_offset_after_leading_whitespace() {
    let text = "  Do the following:\n- Open the file.\n- Close the file.";
    let got = split_sentences(text).unwrap();
    let spans: Vec<(usize, usize, &str)> = got
        .iter()
        .map(|s| (s.start, s.end, s.text.as_str()))
        .collect();
    assert_eq!(
        spans,
        [
            (2, 19, "Do the following:"),
            (20, 36, "- Open the file."),
            (37, 54, "- Close the file."),
        ]
    );
    for s in &got {
        assert_eq!(text.get(s.start..s.end), Some(s.text.as_str()));
    }
}

#[test]
fn spacing_marks_are_word_characters_like_python() {
    // U+0903 is a spacing mark. Python's \w includes it, so the digits after it are not
    // a standalone number and "degrees Celsius" stays two words: 3 in total.
    assert_eq!(count_words("a\u{903}10 degrees Celsius").unwrap(), 3);
}

/// U+A7CE is a capital letter only from Unicode 17. The reference reads Unicode 16, where
/// `str.isupper` is false for it, so "Check it. \u{A7CE} more prose follows." is one
/// sentence there (Python 3.14.8). Rust's own `char::is_uppercase` would split it. The
/// word count is not compared: `regex` reads a later Unicode release than PCRE2, a
/// divergence recorded in tests/fixtures/divergences.md.
#[test]
fn sentence_opener_uppercase_follows_unicode_16_like_python() {
    let got = split_sentences("Check it. \u{A7CE} more prose follows.").unwrap();
    assert_eq!(got.len(), 1);
}
