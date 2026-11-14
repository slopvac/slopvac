//! Escaping for index text that reaches a terminal (threat model N11, O1).

use std::fmt::Write as _;

/// True for characters that can forge or hide terminal output: C0, DEL and C1
/// controls (so newline and tab too), line and paragraph separators, and every
/// default-ignorable or format (Cf) code point of Unicode 15: soft hyphen,
/// bidirectional controls, zero-width characters, Hangul fillers, variation
/// selectors, interlinear annotation marks, tag characters and the like.
fn is_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(
            u32::from(c),
            0xad | 0x34f
                | 0x600..=0x605
                | 0x61c
                | 0x6dd
                | 0x70f
                | 0x890..=0x891
                | 0x8e2
                | 0x115f..=0x1160
                | 0x17b4..=0x17b5
                | 0x180b..=0x180f
                | 0x200b..=0x200f
                | 0x2028..=0x202e
                | 0x2060..=0x206f
                | 0x3164
                | 0xfe00..=0xfe0f
                | 0xfeff
                | 0xffa0
                | 0xfff0..=0xfffb
                | 0x110bd
                | 0x110cd
                | 0x13430..=0x1343f
                | 0x1bca0..=0x1bca3
                | 0x1d173..=0x1d17a
                | 0xe0000..=0xe0fff
        )
}

/// Replaces each unsafe character with `\u{..}`, so text from the index can
/// neither forge nor hide terminal output. Safe text passes unchanged.
#[must_use]
pub fn sanitize_for_terminal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_unsafe(c) {
            let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::sanitize_for_terminal;

    #[test]
    fn safe_text_is_unchanged() {
        assert_eq!(sanitize_for_terminal("Zürich – ok ✓"), "Zürich – ok ✓");
    }

    #[test]
    fn unsafe_characters_become_escapes() {
        let cases = [
            ("a\u{1b}[31mb", "a\\u{1b}[31mb"), // ESC (C0)
            ("a\nb", "a\\u{a}b"),              // newline
            ("a\tb", "a\\u{9}b"),              // tab
            ("a\u{7f}b", "a\\u{7f}b"),         // DEL
            ("a\u{9b}b", "a\\u{9b}b"),         // CSI (C1)
            ("a\u{202e}b", "a\\u{202e}b"),     // right-to-left override
            ("a\u{2067}b", "a\\u{2067}b"),     // right-to-left isolate
            ("a\u{200b}b", "a\\u{200b}b"),     // zero-width space
            ("a\u{feff}b", "a\\u{feff}b"),     // byte order mark
            ("a\u{2028}b", "a\\u{2028}b"),     // line separator
            ("a\u{e0041}b", "a\\u{e0041}b"),   // tag letter A
        ];
        for (input, want) in cases {
            assert_eq!(sanitize_for_terminal(input), want, "{input:?}");
        }
    }
}
