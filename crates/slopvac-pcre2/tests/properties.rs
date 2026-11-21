//! Property tests: arbitrary patterns and subjects never panic, spans are always safe to
//! slice with, and the JIT and the interpreter agree whenever both finish.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use proptest::prelude::*;
use slopvac_pcre2::{Limits, Options, Regex};

/// Small limits keep random catastrophic patterns fast; hitting them is a valid outcome.
const LIMITS: Limits = Limits {
    match_limit: 20_000,
    depth_limit: 2_000,
    heap_limit_kib: 512,
    jit_match_limit: 2_000,
    jit_stack_bytes: 64 * 1024,
};

/// Pattern text drawn from regex metacharacters, letters and a few multi-byte characters,
/// so most inputs are close to valid and exercise groups, classes, lookarounds and
/// repeats; the rest are syntax errors, which must come back as `Err`.
fn pattern() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just("("),
            Just(")"),
            Just("(?:"),
            Just("(?="),
            Just("(?<="),
            Just("(?!"),
            Just("["),
            Just("]"),
            Just("[^"),
            Just("|"),
            Just("*"),
            Just("+"),
            Just("?"),
            Just("{1,3}"),
            Just("{"),
            Just("}"),
            Just("^"),
            Just("$"),
            Just("."),
            Just("\\"),
            Just("\\b"),
            Just("\\w"),
            Just("\\K"),
            Just("\\C"),
            Just("\\1"),
            Just("a"),
            Just("b"),
            Just("é"),
            Just("😀"),
            Just(" "),
        ],
        0..14,
    )
    .prop_map(|parts| parts.concat())
}

fn subject() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[ab é😀\\n]{0,24}").unwrap()
}

proptest! {
    #[test]
    fn arbitrary_patterns_and_subjects_never_panic(
        pattern in pattern(),
        subject in subject(),
        caseless in any::<bool>(),
        ucp in any::<bool>(),
        jit in any::<bool>(),
    ) {
        let options = Options::new().caseless(caseless).ucp(ucp).jit(jit);
        let Ok(re) = Regex::with_limits(&pattern, options, LIMITS) else {
            return Ok(());
        };
        let _ = re.is_match(&subject);
        let _ = re.find(&subject);
        let _ = re.captures(&subject);
        let _ = re.find_all(&subject);
    }

    #[test]
    fn find_all_spans_are_ordered_disjoint_and_sliceable(
        pattern in pattern(),
        subject in subject(),
    ) {
        let Ok(re) = Regex::with_limits(&pattern, Options::new(), LIMITS) else {
            return Ok(());
        };
        let Ok(spans) = re.find_all(&subject) else {
            return Ok(());
        };
        let mut previous_end = 0;
        for span in spans {
            prop_assert!(span.start >= previous_end);
            prop_assert!(span.start <= span.end && span.end <= subject.len());
            // Slicing panics unless both ends are character boundaries.
            let _ = &subject[span.clone()];
            previous_end = span.end;
        }
    }

    #[test]
    fn the_jit_and_the_interpreter_agree_whenever_both_finish(
        pattern in pattern(),
        subject in subject(),
    ) {
        let jit = Regex::with_limits(&pattern, Options::new().jit(true), LIMITS);
        let plain = Regex::with_limits(&pattern, Options::new().jit(false), LIMITS);
        let (Ok(jit), Ok(plain)) = (jit, plain) else {
            return Ok(());
        };
        if let (Ok(a), Ok(b)) = (jit.find_all(&subject), plain.find_all(&subject)) {
            prop_assert_eq!(a, b);
        }
    }

    #[test]
    fn arbitrary_bytes_as_pattern_text_never_panic(
        pattern in "\\PC{0,24}",
        subject in subject(),
    ) {
        if let Ok(re) = Regex::with_limits(&pattern, Options::new().ucp(true), LIMITS) {
            let _ = re.find_all(&subject);
        }
    }
}
