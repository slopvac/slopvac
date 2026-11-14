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
