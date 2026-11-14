//! Token and span behaviour through the public interface: affix handling,
//! UTF-8 byte offsets.

use slopvac_pos::Tagger;

fn tokens(text: &str) -> Vec<(&str, usize)> {
    Tagger::english()
        .tag_text(text)
        .into_iter()
        .map(|t| {
            assert_eq!(&text[t.span.clone()], t.text, "span matches text");
            (t.text, t.span.start)
        })
        .collect()
}

/// Every opening and closing punctuation mark prose v3.3.1 peels off a word,
/// including the German, Finnish, Spanish and CJK forms.
#[test]
fn every_prose_prefix_and_suffix_is_peeled_off() {
    let prefixes = [
        "$", "(", "\"", "[", "\u{201c}", "\u{2018}", "\u{ab}", "\u{201e}", "\u{201d}", "\u{201a}",
        "\u{2039}", "\u{bf}", "\u{a1}", "\u{ff08}", "\u{300c}", "\u{300e}", "\u{3010}", "\u{300a}",
        "\u{3008}", "\u{3014}",
    ];
    let suffixes = [
        ",", ")", "\"", "]", "!", ";", ".", "?", ":", "'", "\u{201d}", "\u{2019}", "\u{bb}",
        "\u{201c}", "\u{2018}", "\u{203a}", "\u{2026}", "\u{3002}", "\u{ff0c}", "\u{3001}",
        "\u{ff01}", "\u{ff1f}", "\u{ff1a}", "\u{ff1b}", "\u{ff09}", "\u{300d}", "\u{300f}",
        "\u{3011}", "\u{300b}", "\u{3009}", "\u{3015}",
    ];
    assert_eq!(prefixes.len(), 20);
    assert_eq!(suffixes.len(), 31);
    for p in prefixes {
        let text = format!("{p}word");
        assert_eq!(tokens(&text), [(p, 0), ("word", p.len())], "prefix {p:?}");
    }
    for s in suffixes {
        let text = format!("word{s}");
        assert_eq!(tokens(&text), [("word", 0), (s, 4)], "suffix {s:?}");
    }
}

#[test]
fn spans_are_utf8_byte_offsets_around_multibyte_text() {
    let text = "Caf\u{e9} \u{2039}na\u{ef}ve\u{203a} idea";
    assert_eq!(
        tokens(text),
        [
            ("Caf\u{e9}", 0),
            ("\u{2039}", 6),
            ("na\u{ef}ve", 9),
            ("\u{203a}", 15),
            ("idea", 19),
        ]
    );
}
