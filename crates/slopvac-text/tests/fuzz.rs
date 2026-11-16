//! `Pattern::new` takes rule data from files, so no input may panic it: a pattern either
//! compiles or is an error, and a compiled pattern either matches or hits a limit.
//!
//! Set `PROPTEST_CASES` to change the case count (default 512).

use proptest::prelude::*;
use slopvac_text::Pattern;

/// Pieces that stress the tokenizer: every construct it reads, and the stray characters
/// that open or close them.
const PIECES: &[&str] = &[
    r"\w",
    r"\W",
    r"\b",
    r"\B",
    r"\d",
    r"\\",
    r"\",
    "[",
    "]",
    "[^",
    "[]",
    "[^]",
    "[:",
    ":]",
    "[:alpha:]",
    "[:^digit:]",
    "(",
    ")",
    "(?",
    "(?x)",
    "(?-x)",
    "(?x:",
    "(?i)",
    "(?^)",
    "(?#",
    "(?<n>",
    "(?P<n>",
    "(?'n'",
    "(?|",
    "(?>",
    "(?=",
    "(?!",
    "(?<=a)",
    "(?<!a)",
    "(*",
    "(*MARK:",
    "(*UCP)",
    "(*pla:",
    "(*positive_lookahead:",
    "(*",
    "#",
    "\n",
    " ",
    r"\Q",
    r"\E",
    r"\c",
    r"\cA",
    r"\x",
    r"\x4",
    r"\x{41}",
    r"\x{",
    r"\0",
    r"\01",
    r"\101",
    r"\8",
    r"\p{L}",
    r"\p{",
    r"\P{L}",
    r"\N{U+41}",
    r"\o{101}",
    r"\g{1}",
    r"\k{n}",
    "-",
    "^",
    "$",
    "|",
    "?",
    "*",
    "+",
    "{",
    "}",
    "{1,2}",
    ":",
    "a",
    "b",
    "A",
    "0",
    "_",
    "\u{903}",
    "\u{200d}",
    "\u{130}",
];

fn soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..24).prop_map(|v| v.concat())
}

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(
            &[
                "a", "b", "A", "0", "_", "-", " ", "[", "#", "\n", "\u{903}", "\u{200d}",
            ][..],
        ),
        0..8,
    )
    .prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(512)
    ))]

    #[expect(
        clippy::let_underscore_must_use,
        reason = "the property is that the call returns without panicking; Ok and Err both pass"
    )]
    #[test]
    fn arbitrary_text_never_panics_the_compiler(source in ".{0,60}", caseless in any::<bool>()) {
        let _ = Pattern::new(&source, caseless);
    }

    #[expect(
        clippy::let_underscore_must_use,
        reason = "the property is that the call returns without panicking; Ok and Err both pass"
    )]
    #[test]
    fn a_soup_of_pattern_tokens_never_panics_the_compiler(source in soup(), caseless in any::<bool>()) {
        let _ = Pattern::new(&source, caseless);
    }

    #[expect(
        clippy::let_underscore_must_use,
        reason = "the property is that the call returns without panicking; Ok and Err both pass"
    )]
    #[test]
    fn a_compiled_soup_pattern_never_panics_the_matcher(
        source in soup(),
        subject in text(),
        caseless in any::<bool>(),
    ) {
        if let Ok(pattern) = Pattern::new(&source, caseless) {
            let _ = pattern.find_all(&subject);
            let _ = pattern.is_match(&subject);
        }
    }
}
