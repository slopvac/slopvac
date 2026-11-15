//! The limits policy: a limit hit is a typed error, the JIT retries on the interpreter,
//! and neither depends on wall-clock time. Each test asserts the error, not how long it
//! took to arrive.

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
fn a_spent_jit_budget_is_retried_on_the_interpreter() {
    // A one-step JIT budget cannot finish even this easy match. The retry on the
    // interpreter (policy limits) must produce the real answer, not an error.
    let limits = Limits {
        jit_match_limit: 1,
        ..Limits::POLICY
    };
    let re = Regex::with_limits(r"(\w+) (\w+)", Options::new(), limits).unwrap();
    assert_eq!(re.find("hello big world").unwrap(), Some(0..9));
    assert_eq!(re.find_all("a b c d").unwrap(), [0..3, 4..7]);
}

#[test]
fn an_exhausted_jit_stack_is_retried_on_the_interpreter() {
    // Nested groups in a long repetition need more machine stack than 1 KiB allows.
    let limits = Limits {
        jit_stack_bytes: 1024,
        ..Limits::POLICY
    };
    let re = Regex::with_limits(r"^(?:(a)|(b))*$", Options::new(), limits).unwrap();
    assert_eq!(re.find(&"ab".repeat(5_000)).unwrap(), Some(0..10_000));
}

#[test]
fn the_jit_and_the_interpreter_agree_after_a_retry() {
    let tight = Limits {
        jit_match_limit: 1,
        ..Limits::POLICY
    };
    let subject = "one two  three\nfour";
    let pattern = r"\b\w+\b";
    let retried = Regex::with_limits(pattern, Options::new(), tight).unwrap();
    let plain = interpreter_only(pattern, Limits::POLICY);
    assert_eq!(
        retried.find_all(subject).unwrap(),
        plain.find_all(subject).unwrap()
    );
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
