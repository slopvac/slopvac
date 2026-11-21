//! The limits policy: a limit hit is a typed error, the JIT retries on the interpreter,
//! and neither depends on wall-clock time. Each test asserts the error, not how long it
//! took to arrive.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use slopvac_pcre2::{Limits, MatchError, Options, Regex};

const CATASTROPHIC: &str = r"^(a|aa)+$";

/// 64 `a`s then `b`: the alternation can split the run in about 2^40 ways before failing.
fn catastrophic_subject() -> String {
    format!("{}b", "a".repeat(64))
}

/// `rhetorical-question-transition` from `ai-tells-structure`, the pattern that took 597 s
/// under the JIT alone on 1 MB of spaces (slopvac-8l4.3.20).
const RHETORICAL_QUESTION: &str =
    r"(?m)^\s*(?:#{1,6}\s*)?(?:So|But|Now|And)?\s*(?:why|what|how|who)\b[^.!?\n]{0,60}\?\s*$";

fn interpreter_only(pattern: &str, limits: Limits) -> Regex {
    Regex::with_limits(pattern, Options::new().jit(false), limits).unwrap()
}

fn jit(pattern: &str, limits: Limits) -> Regex {
    Regex::with_limits(pattern, Options::new().jit(true), limits).unwrap()
}

#[test]
fn the_policy_values_are_the_recorded_ones() {
    let policy = Limits::POLICY;
    assert_eq!(policy.match_limit, 10_000_000);
    assert_eq!(policy.depth_limit, 10_000_000);
    assert_eq!(policy.heap_limit_kib, 256 * 1024);
    assert_eq!(policy.jit_match_limit, 100_000);
    assert_eq!(policy.jit_stack_bytes, 8 * 1024 * 1024);
    assert_eq!(Limits::default(), policy);
}

#[test]
fn catastrophic_backtracking_stops_at_the_match_limit_under_the_policy() {
    let re = Regex::new(CATASTROPHIC, Options::new()).unwrap();
    assert_eq!(
        re.is_match(&catastrophic_subject()),
        Err(MatchError::MatchLimit { limit: 10_000_000 })
    );
}

#[test]
fn find_all_find_and_captures_report_the_same_limit() {
    let re = Regex::new(CATASTROPHIC, Options::new()).unwrap();
    let subject = catastrophic_subject();
    let limit = MatchError::MatchLimit { limit: 10_000_000 };
    assert_eq!(re.find_all(&subject), Err(limit.clone()));
    assert_eq!(re.find(&subject), Err(limit.clone()));
    assert_eq!(re.captures(&subject), Err(limit));
}

#[test]
fn the_interpreter_alone_hits_the_same_match_limit() {
    let re = interpreter_only(CATASTROPHIC, Limits::POLICY);
    assert_eq!(
        re.is_match(&catastrophic_subject()),
        Err(MatchError::MatchLimit { limit: 10_000_000 })
    );
}

#[test]
fn the_rhetorical_question_pattern_on_a_megabyte_of_spaces_is_bounded() {
    let re = Regex::new(RHETORICAL_QUESTION, Options::new().caseless(true).ucp(true)).unwrap();
    let spaces = " ".repeat(1 << 20);
    // The interpreter limit may stop it or the scan may finish; both are bounded. It must
    // not panic, and a failure must be a limit error.
    match re.find_all(&spaces) {
        Ok(found) => assert_eq!(found, []),
        Err(e) => assert!(e.is_limit(), "{e:?}"),
    }
}

#[test]
fn a_low_match_limit_is_reported_with_its_value() {
    let limits = Limits {
        match_limit: 1_000,
        ..Limits::POLICY
    };
    let re = interpreter_only(CATASTROPHIC, limits);
    assert_eq!(
        re.is_match(&catastrophic_subject()),
        Err(MatchError::MatchLimit { limit: 1_000 })
    );
}

#[test]
fn a_low_depth_limit_is_reported_as_a_depth_error() {
    let limits = Limits {
        depth_limit: 5,
        ..Limits::POLICY
    };
    // Each repetition of the group nests one backtracking frame.
    let re = interpreter_only(r"^(?:a|b)*$", limits);
    assert_eq!(
        re.is_match(&format!("{}!", "ab".repeat(100))),
        Err(MatchError::DepthLimit { limit: 5 })
    );
}

#[test]
fn a_low_heap_limit_is_reported_as_a_heap_error() {
    let limits = Limits {
        heap_limit_kib: 1,
        ..Limits::POLICY
    };
    let re = interpreter_only(r"^(?:a|b)*$", limits);
    assert_eq!(
        re.is_match(&format!("{}!", "ab".repeat(100_000))),
        Err(MatchError::HeapLimit { kib: 1 })
    );
}

#[test]
fn limit_errors_are_limits_and_other_errors_are_not() {
    assert!(MatchError::MatchLimit { limit: 1 }.is_limit());
    assert!(MatchError::DepthLimit { limit: 1 }.is_limit());
    assert!(MatchError::HeapLimit { kib: 1 }.is_limit());
    assert!(!MatchError::OutOfMemory.is_limit());
    assert!(!MatchError::InvalidOffsets.is_limit());
}

#[test]
fn regexes_with_different_limits_do_not_share_a_stale_context() {
    // Alternate on one thread: the cached per-thread contexts must follow the regex.
    let tight = Limits {
        match_limit: 1_000,
        ..Limits::POLICY
    };
    let strict = interpreter_only(CATASTROPHIC, tight);
    let policy = interpreter_only(CATASTROPHIC, Limits::POLICY);
    let subject = catastrophic_subject();
    for _ in 0..3 {
        assert_eq!(
            strict.is_match(&subject),
            Err(MatchError::MatchLimit { limit: 1_000 })
        );
        assert_eq!(
            policy.is_match(&subject),
            Err(MatchError::MatchLimit { limit: 10_000_000 })
        );
    }
}

#[test]
fn a_limit_error_on_one_thread_leaves_the_others_unaffected() {
    let heavy = Regex::new(CATASTROPHIC, Options::new()).unwrap();
    let light = Regex::new(r"\d+", Options::new()).unwrap();
    let subject = catastrophic_subject();
    std::thread::scope(|scope| {
        scope.spawn(|| assert!(heavy.is_match(&subject).unwrap_err().is_limit()));
        scope.spawn(|| {
            for _ in 0..1_000 {
                assert_eq!(light.find("ab 12").unwrap(), Some(3..5));
            }
        });
    });
}

/// Overlapping greedy repetitions with a failing suffix, on a long run that matches the
/// repetitions but never the suffix. The options switch off the start optimisations that
/// would reject the subject at once. Backtracking is quadratic in the subject length; under
/// the JIT that work does not count against the JIT match limit.
const QUADRATIC: &str = r"(*NO_AUTO_POSSESS)(*NO_START_OPT)a*a*b";

#[test]
fn quadratic_backtracking_returns_a_limit_error_on_the_default_engine() {
    let re = Regex::new(QUADRATIC, Options::new()).unwrap();
    assert!(!re.is_jit());
    // About 20,000^2 / 2 = 2e8 steps without a limit; the policy stops at 1e7.
    let subject = "a".repeat(20_000);
    assert_eq!(
        re.is_match(&subject),
        Err(MatchError::MatchLimit { limit: 10_000_000 })
    );
    assert_eq!(
        re.find_all(&subject),
        Err(MatchError::MatchLimit { limit: 10_000_000 })
    );
}

#[test]
fn the_default_engine_is_the_interpreter_for_every_pattern() {
    for pattern in [r"\d+", CATASTROPHIC, QUADRATIC] {
        assert!(!Regex::new(pattern, Options::new()).unwrap().is_jit());
    }
}

#[test]
fn a_no_jit_pattern_under_the_jit_option_runs_under_the_interpreter_limits() {
    // `(*NO_JIT)` makes JIT compilation "succeed" with no JIT code. The wrapper must see
    // that, so the heap limit applies instead of PCRE2's default of about 20 GB.
    let limits = Limits {
        heap_limit_kib: 1,
        ..Limits::POLICY
    };
    let re = jit(r"(*NO_JIT)^(?:a|b)*$", limits);
    assert!(!re.is_jit());
    assert_eq!(
        re.is_match(&format!("{}!", "ab".repeat(100_000))),
        Err(MatchError::HeapLimit { kib: 1 })
    );
}
