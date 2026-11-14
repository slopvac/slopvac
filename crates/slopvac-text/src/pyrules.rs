//! Python-compatible character predicates. The reference is Python's `str` methods, so
//! Rust's differ at the edges: whitespace, casefolding and `isdigit`.

/// `str.isspace` / `str.strip` whitespace: Unicode `White_Space` plus U+001C to U+001F.
pub(crate) fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(is_space)
}

pub(crate) fn trim_start(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

/// `str.casefold` for the characters whose full case folding differs from lowercase.
pub(crate) fn casefold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'ſ' => out.push('s'),
            'ß' | 'ẞ' => out.push_str("ss"),
            'ς' => out.push('σ'),
            _ => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// `str.isdigit` for one character: decimal digits plus the other characters whose
/// `Numeric_Type` is Digit (superscripts, subscripts, circled and dingbat digits). Vulgar
/// fractions such as ¼ are not digits.
pub(crate) fn is_digit(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_digit();
    }
    matches!(
        c as u32,
        0xB2 | 0xB3 | 0xB9
            | 0x2070
            | 0x2074..=0x2079
            | 0x2080..=0x2089
            | 0x2460..=0x2468
            | 0x2474..=0x247C
            | 0x2488..=0x2490
            | 0x24EA
            | 0x24F5..=0x24FD
            | 0x24FF
            | 0x2776..=0x277E
            | 0x2780..=0x2788
            | 0x278A..=0x2792
            | 0x1F100..=0x1F10A
    ) || nd_block(c)
}

/// Unicode decimal digits (Nd) outside ASCII, as runs of ten.
fn nd_block(c: char) -> bool {
    const STARTS: &[u32] = &[
        0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66, 0xDE6,
        0xE50, 0xED0, 0xF20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90,
        0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0,
        0xFF10, 0x104A0, 0x11066, 0x1D7CE, 0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6,
    ];
    let v = c as u32;
    STARTS.iter().any(|&s| (s..s + 10).contains(&v))
}
