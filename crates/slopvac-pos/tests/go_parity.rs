//! Tags that prose v3.3.1 itself produces (Go 1.27, `tag.New()` with the
//! default tokenizer) for text whose casing and character classes differ
//! between Go's `unicode` tables and Rust's Unicode properties: roman
//! numerals and circled letters are not Lu/Ll in Go, and a titlecase letter
//! is neither upper nor lower. Expected values come from running prose.

use slopvac_pos::Tagger;

fn tags(text: &str) -> Vec<(&str, &str)> {
    Tagger::english()
        .tag_text(text)
        .into_iter()
        .map(|t| (t.text, t.tag))
        .collect()
}

#[test]
fn matches_prose_on_go_casing_edge_cases() {
    let cases: [(&str, &[(&str, &str)]); 7] = [
        (
            "Read Chapter \u{2163} first.",
            &[
                ("Read", "VB"),
                ("Chapter", "NNP"),
                ("\u{2163}", "NNP"),
                ("first", "RB"),
                (".", "."),
            ],
        ),
        (
            "See section \u{24b6} for details.",
            &[
                ("See", "VB"),
                ("section", "NN"),
                ("\u{24b6}", "''"),
                ("for", "IN"),
                ("details", "NNS"),
                (".", "."),
            ],
        ),
        (
            "The \u{1c5}ungla is big.",
            &[
                ("The", "DT"),
                ("\u{1c5}ungla", "NN"),
                ("is", "VBZ"),
                ("big", "JJ"),
                (".", "."),
            ],
        ),
        (
            "Use \u{24d0} and \u{2160}\u{2161} here.",
            &[
                ("Use", "VB"),
                ("\u{24d0}", ","),
                ("and", "CC"),
                ("\u{2160}\u{2161}", "NNP"),
                ("here", "RB"),
                (".", "."),
            ],
        ),
        (
            "Price is 5\u{b2} now.",
            &[
                ("Price", "NN"),
                ("is", "VBZ"),
                ("5\u{b2}", "CD"),
                ("now", "RB"),
                (".", "."),
            ],
        ),
        (
            "Open \u{2167} and \u{2168}.",
            &[
                ("Open", "VB"),
                ("\u{2167}", "NNP"),
                ("and", "CC"),
                ("\u{2168}", "NNP"),
                (".", "."),
            ],
        ),
        (
            "The ABC\u{2163} file works.",
            &[
                ("The", "DT"),
                ("ABC\u{2163}", "NN"),
                ("file", "NN"),
                ("works", "VBZ"),
                (".", "."),
            ],
        ),
    ];
    for (text, want) in cases {
        assert_eq!(tags(text), want, "{text}");
    }
}

/// A sentence starts after `.` unless the next word starts with a lowercase
/// letter, and "lowercase" is Go's `Ll`: U+24D0 (circled small a) is a symbol,
/// not `Ll`, so it starts a new sentence and the tagger sees fresh context.
/// Expected tags: prose tags "The file ." and "ⓐ key works" as two sentences;
/// as one sentence it would tag the circled letter `,` instead of `NFP`.
#[test]
fn a_circled_letter_after_a_full_stop_starts_a_new_sentence() {
    assert_eq!(
        tags("The file. \u{24d0} key works"),
        [
            ("The", "DT"),
            ("file", "NN"),
            (".", "."),
            ("\u{24d0}", "NFP"),
            ("key", "JJ"),
            ("works", "NNS"),
        ]
    );
}
