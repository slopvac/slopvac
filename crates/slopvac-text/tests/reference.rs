#![allow(missing_docs, clippy::cast_possible_truncation)]

use serde_json::Value;
use slopvac_text::{TextType, classify_text_type, count_words, split_sentences};

fn cases() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!("fixtures/reference.json"))
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn kind(name: &str) -> TextType {
    serde_json::from_value(Value::String(name.to_owned())).unwrap()
}

#[test]
fn word_counts_match_the_python_reference() {
    for c in cases() {
        let text = c["text"].as_str().unwrap();
        assert_eq!(
            count_words(text),
            c["words"].as_u64().unwrap() as usize,
            "{text:?}"
        );
    }
}

#[test]
fn text_types_match_the_python_reference() {
    for c in cases() {
        let text = c["text"].as_str().unwrap();
        assert_eq!(
            classify_text_type(text),
            kind(c["text_type"].as_str().unwrap()),
            "{text:?}"
        );
    }
}

#[test]
fn sentences_match_the_python_reference() {
    for c in cases() {
        let text = c["text"].as_str().unwrap();
        let got = split_sentences(text);
        let want = c["sentences"].as_array().unwrap();
        assert_eq!(
            got.len(),
            want.len(),
            "sentence count for {text:?}: {got:?}"
        );
        for (g, w) in got.iter().zip(want) {
            assert_eq!(g.text, w["text"].as_str().unwrap(), "{text:?}");
            assert_eq!(g.start, w["start"].as_u64().unwrap() as usize, "{text:?}");
            assert_eq!(g.end, w["end"].as_u64().unwrap() as usize, "{text:?}");
            assert_eq!(
                g.word_count,
                w["word_count"].as_u64().unwrap() as usize,
                "{text:?}"
            );
            assert_eq!(
                g.text_type,
                kind(w["text_type"].as_str().unwrap()),
                "{text:?}"
            );
            assert_eq!(&text[g.start..g.end], g.text);
        }
    }
}
