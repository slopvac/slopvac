//! Matching semantics through the public API. Expected spans are worked out by hand from
//! Python's `re.finditer` rules (the empty-match cases follow `CPython` 3.7+).
#![allow(clippy::single_range_in_vec_init)]

use rstest::rstest;
use slopvac_pcre2::{CompileError, Options, Regex};

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern, Options::new()).unwrap()
}

#[rstest]
// An empty match at every position, including the end.
#[case("x*", "ab", &[0..0, 1..1, 2..2])]
// An empty match may follow a non-empty one: Python gives '', 'aa', '', ''.
#[case("a*", "baac", &[0..0, 1..3, 3..3, 4..4])]
// Empty matches fall only on character boundaries: "é" is two bytes.
#[case("", "é", &[0..0, 2..2])]
#[case("x*", "", &[0..0])]
#[case(r"\b", "ab cd", &[0..0, 2..2, 3..3, 5..5])]
#[case(r"\d+", "a12b345", &[1..3, 4..7])]
#[case("é+", "aééb", &[1..5])]
#[case("zzz", "abc", &[])]
fn find_all_follows_python_finditer(
    #[case] pattern: &str,
    #[case] subject: &str,
    #[case] expected: &[std::ops::Range<usize>],
) {
    assert_eq!(regex(pattern).find_all(subject).unwrap(), expected);
}

#[test]
fn find_returns_the_first_match_only() {
    assert_eq!(regex("b+").find("abbcbb").unwrap(), Some(1..3));
    assert_eq!(regex("q").find("abc").unwrap(), None);
}

#[test]
fn is_match_does_not_anchor() {
    assert!(regex("bc").is_match("abcd").unwrap());
    assert!(!regex("^bc").is_match("abcd").unwrap());
}

#[test]
fn captures_report_every_group_with_unset_ones_as_none() {
    // Group 1 is "a" at 1..2, group 2 did not take part.
    let caps = regex("(a)(b)?").captures("xac").unwrap().unwrap();
    assert_eq!(caps.len(), 3);
    assert_eq!(caps.get(0), Some(1..2));
    assert_eq!(caps.get(1), Some(1..2));
    assert_eq!(caps.get(2), None);
    assert_eq!(caps.get(3), None);
}

#[test]
fn captures_of_a_missing_match_is_none() {
    assert_eq!(regex("(a)").captures("xyz").unwrap(), None);
}

#[test]
fn ucp_changes_what_a_word_character_is() {
    let plain = Regex::new(r"\w+", Options::new()).unwrap();
    let ucp = Regex::new(r"\w+", Options::new().ucp(true)).unwrap();
    // "é" is two bytes. Without UCP only ASCII letters are word characters.
    assert_eq!(plain.find("éa").unwrap(), Some(2..3));
    assert_eq!(ucp.find("éa").unwrap(), Some(0..3));
}

#[test]
fn caseless_folds_non_ascii_letters_in_utf_mode() {
    let re = Regex::new("é", Options::new().caseless(true)).unwrap();
    assert_eq!(re.find("xÉ").unwrap(), Some(1..3));
}

#[test]
fn a_pattern_may_contain_a_nul_byte() {
    assert_eq!(regex("a\0b").find("xa\0b").unwrap(), Some(1..4));
}

#[test]
fn a_syntax_error_reports_the_offset_and_text() {
    let err = Regex::new("ab(cd", Options::new()).unwrap_err();
    let CompileError::Syntax { message, offset } = err else {
        panic!("expected a syntax error, got {err:?}");
    };
    assert_eq!(offset, 5);
    assert!(message.contains("missing closing parenthesis"), "{message}");
}

#[test]
fn bounded_variable_length_lookbehind_compiles() {
    // Needs PCRE2 10.43 or newer; the version check passes only for such a build.
    let re = regex(r"(?<=a{1,3}|bc)x");
    assert_eq!(re.find_all("aax bcx dx").unwrap(), [2..3, 6..7]);
}

#[test]
fn clones_share_one_compiled_pattern() {
    let re = regex("a+");
    let copy = re.clone();
    drop(re);
    assert_eq!(copy.find("baa").unwrap(), Some(1..3));
}

#[test]
fn a_regex_is_usable_from_many_threads_at_once() {
    let re = regex(r"(\w+)@(\w+)\.com");
    std::thread::scope(|scope| {
        for n in 0..8 {
            let re = &re;
            scope.spawn(move || {
                for i in 0..200 {
                    let subject = format!("mail user{n}x{i}@host{n}.com now");
                    let found = re.find(&subject).unwrap().unwrap();
                    assert_eq!(&subject[found], format!("user{n}x{i}@host{n}.com"));
                }
            });
        }
    });
}

#[rstest]
#[case(r"\C")]
#[case(r"\C\K\C")]
#[case(r"a(?:\C|b)")]
#[case(r"(?=\C)x")]
fn one_byte_matching_is_refused_at_compile_time(#[case] pattern: &str) {
    // `\C` takes one byte of a multi-byte character, so a span could end inside it.
    let err = Regex::new(pattern, Options::new()).unwrap_err();
    assert!(
        matches!(err, CompileError::BackslashC { .. }),
        "{pattern}: {err:?}"
    );
}

#[test]
fn a_quoted_backslash_c_is_a_literal_and_still_compiles() {
    assert_eq!(regex(r"\Q\C\E").find(r"a\Cb").unwrap(), Some(1..3));
}

#[test]
fn k_resets_the_start_on_a_character_boundary() {
    // `\K` moves the reported start; with whole-character matching it lands between
    // characters, so the span is safe to slice.
    let found = regex(r"caf\K.").find("café!").unwrap().unwrap();
    assert_eq!(found, 3..5);
    assert_eq!(&"café!"[found], "é");
}
