//! Python-compatible character predicates. The reference is Python's `str` methods, so
//! Rust's differ at the edges: whitespace, casefolding and `isdigit`.

use std::cmp::Ordering;

use crate::unicode_data::{CASEFOLD, DIGITS};

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

/// `str.casefold`: full Unicode case folding, from tables generated out of Python itself
/// (see `unicode_data.rs`).
pub(crate) fn casefold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match CASEFOLD.binary_search_by_key(&c, |&(k, _)| k) {
            Ok(i) => out.push_str(CASEFOLD[i].1),
            Err(_) => out.push(c),
        }
    }
    out
}

/// `str.isdigit` for one character: `Numeric_Type` Digit or Decimal. Vulgar fractions such
/// as ¼ are not digits.
pub(crate) fn is_digit(c: char) -> bool {
    let v = c as u32;
    DIGITS
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
