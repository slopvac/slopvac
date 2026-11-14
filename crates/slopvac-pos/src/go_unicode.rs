//! Character classes as Go's `unicode` package defines them, which is what
//! prose v3.3.1 uses. Rust's `char` predicates differ: `is_uppercase` also
//! takes `Other_Uppercase` (roman numerals, circled letters), `is_numeric` takes
//! No and Nl, and a titlecase letter such as U+01C5 is neither Lu nor Ll.
//!
//! The tables are generated from Go by `tools/gen_unicode_tables.py`.

use crate::unicode_tables::{DIGIT, LOWER, PUNCT_OR_SYMBOL, UPPER};

fn in_table(table: &[(u32, u32)], c: char) -> bool {
    let c = u32::from(c);
    let idx = table.partition_point(|&(_, hi)| hi < c);
    table.get(idx).is_some_and(|&(lo, _)| lo <= c)
}

/// `unicode.IsUpper`: category Lu.
pub(crate) fn is_upper(c: char) -> bool {
    in_table(&UPPER, c)
}

/// `unicode.IsLower`: category Ll.
pub(crate) fn is_lower(c: char) -> bool {
    in_table(&LOWER, c)
}

/// `unicode.IsDigit`: category Nd.
pub(crate) fn is_digit(c: char) -> bool {
    in_table(&DIGIT, c)
}

/// `unicode.IsPunct(c) || unicode.IsSymbol(c)`: categories P and S.
pub(crate) fn is_punct_or_symbol(c: char) -> bool {
    in_table(&PUNCT_OR_SYMBOL, c)
}

/// `unicode.ToLower`. Rust's `char::to_lowercase` yields a full mapping whose
/// first character is Go's simple mapping for every code point (checked
/// against Go 1.27.4 over all of Unicode), so taking it keeps Go's result.
pub(crate) fn to_lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values from Go 1.27.4 (`unicode.IsUpper` and friends).
    #[test]
    fn classes_follow_go_not_rust_properties() {
        // (char, upper, lower, digit, punct_or_symbol)
        for (c, up, lo, dg, ps) in [
            ('A', true, false, false, false),
            ('z', false, true, false, false),
            ('\u{2163}', false, false, false, false), // roman numeral four, Nl
            ('\u{24b6}', false, false, false, true),  // circled A, So
            ('\u{24d0}', false, false, false, true),  // circled a, So
            ('\u{1c5}', false, false, false, false),  // titlecase Dz, Lt
            ('\u{b2}', false, false, false, false),   // superscript two, No
            ('\u{663}', false, false, true, false),   // Arabic-Indic three, Nd
            ('!', false, false, false, true),
            ('$', false, false, false, true),
            ('\u{301}', false, false, false, false), // combining acute, Mn
        ] {
            assert_eq!(is_upper(c), up, "{c:?} upper");
            assert_eq!(is_lower(c), lo, "{c:?} lower");
            assert_eq!(is_digit(c), dg, "{c:?} digit");
            assert_eq!(is_punct_or_symbol(c), ps, "{c:?} punct or symbol");
        }
    }

    #[test]
    fn decimal_digit_runs_end_exactly() {
        assert!(is_digit('0') && is_digit('9'));
        assert!(!is_digit('/') && !is_digit(':'));
        assert!(is_digit('\u{669}') && !is_digit('\u{66a}'));
        assert!(is_digit('\u{1fbf9}') && !is_digit('\u{1fbfa}'));
    }
}
