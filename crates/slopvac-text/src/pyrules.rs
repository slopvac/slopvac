//! Python-compatible character predicates. The reference is Python's `str` methods, so
//! Rust's differ at the edges: whitespace, case mapping and `isdigit`. Each predicate
//! reads a table generated from Python itself.

use std::cmp::Ordering;

use crate::unicode_data::{
    CASE_IGNORABLE, CASED, CASEFOLD, DIGITS, LOWER, SPACE, UPPER, UPPERCASE,
};

fn in_ranges(ranges: &[(u32, u32)], c: char) -> bool {
    let v = u32::from(c);
    ranges
        .binary_search_by(|&(lo, hi)| {
            if hi < v {
                Ordering::Less
            } else if lo > v {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .is_ok()
}

fn mapped(table: &'static [(char, &'static str)], c: char) -> Option<&'static str> {
    table
        .binary_search_by_key(&c, |&(k, _)| k)
        .ok()
        .and_then(|i| table.get(i))
        .map(|&(_, m)| m)
}

/// `str.isspace` for one character. This is Unicode `White_Space` plus U+001C to U+001F,
/// read from Python's tables. Use it where the reference calls `str.isspace`, `strip`,
/// `lstrip` or `split()`. For `\s` in a pattern use [`regex_space`].
pub(crate) fn py_isspace(c: char) -> bool {
    in_ranges(SPACE, c)
}

/// `str.strip()`.
pub(crate) fn py_strip(s: &str) -> &str {
    s.trim_matches(py_isspace)
}

/// `str.lstrip()`.
pub(crate) fn py_lstrip(s: &str) -> &str {
    s.trim_start_matches(py_isspace)
}

/// The `regex` module's `\s` for one character: `White_Space`, which leaves out the
/// U+001C to U+001F that `str.isspace` accepts. Checked against `regex` 2026.11.9 over
/// every code point. Use it where the reference matches `\s` and the Rust code works on
/// characters instead of a pattern.
pub(crate) fn regex_space(c: char) -> bool {
    py_isspace(c) && !('\u{1c}'..='\u{1f}').contains(&c)
}

/// Map each character through `table`, keeping those it does not list.
fn map_chars(s: &str, table: &'static [(char, &'static str)]) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match mapped(table, c) {
            Some(m) => out.push_str(m),
            None => out.push(c),
        }
    }
    out
}

/// `str.casefold`: full Unicode case folding, from tables generated out of Python itself
/// (see `unicode_data.rs`).
pub(crate) fn casefold(s: &str) -> String {
    map_chars(s, CASEFOLD)
}

/// `str.upper`: full case mapping, so `ﬆ` becomes `ST`.
pub(crate) fn upper(s: &str) -> String {
    map_chars(s, UPPER)
}

/// `str.lower`: full case mapping, with Python's final-sigma rule. A capital sigma after
/// a cased letter and before no cased letter becomes `ς`, skipping case-ignorable
/// characters on both sides.
pub(crate) fn lower(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == '\u{3a3}' {
            let before = chars
                .get(..i)
                .unwrap_or_default()
                .iter()
                .rev()
                .find(|&&p| !in_ranges(CASE_IGNORABLE, p));
            let after = chars
                .get(i + 1..)
                .unwrap_or_default()
                .iter()
                .find(|&&n| !in_ranges(CASE_IGNORABLE, n));
            let is_final = before.is_some_and(|&p| in_ranges(CASED, p))
                && !after.is_some_and(|&n| in_ranges(CASED, n));
            out.push(if is_final { '\u{3c2}' } else { '\u{3c3}' });
        } else {
            match mapped(LOWER, c) {
                Some(m) => out.push_str(m),
                None => out.push(c),
            }
        }
    }
    out
}

/// One-character `str.isupper`: an uppercase letter. Titlecase digraphs such as `ǅ` are not.
pub(crate) fn is_upper(c: char) -> bool {
    in_ranges(UPPERCASE, c)
}

/// `str.isdigit` for one character: `Numeric_Type` Digit or Decimal. Vulgar fractions such
/// as ¼ are not digits.
pub(crate) fn is_digit(c: char) -> bool {
    in_ranges(DIGITS, c)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values come from Python 3.14.8 `str.lower()` and `str.upper()`.
    #[test]
    fn lower_applies_the_final_sigma_rule_like_python() {
        assert_eq!(lower("ΑΣ"), "ας");
        assert_eq!(lower("ΑΣ Β"), "ας β");
        assert_eq!(lower("ΣΑ"), "σα");
        assert_eq!(lower("Α.Σ"), "α.ς");
        assert_eq!(lower("ΑΣ.Β"), "ασ.β");
        assert_eq!(lower("Σ"), "σ");
    }

    #[test]
    fn case_mappings_are_full_like_python() {
        assert_eq!(lower("İ"), "i\u{307}");
        assert_eq!(upper("ﬆß"), "STSS");
        assert_eq!(upper("ǅ"), "Ǆ");
        assert_eq!(lower("ǅ"), "ǆ");
    }
}
