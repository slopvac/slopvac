//! The opt-in JIT: its own match budget and stack, and the retry on the interpreter.
//!
//! Probing technique: the interpreter limit is made deliberately tighter than the JIT's, so
//! an error carrying the interpreter's limit proves the JIT gave up and the call was redone
//! on the interpreter, while `Ok` proves the JIT finished inside its own budget.
//!
//! These tests need real JIT code. On targets that have a JIT (x86-64 and `AArch64` on macOS
//! and glibc Linux) a missing JIT fails the test; elsewhere the test prints why it did not
//! run and returns.
#![allow(
    clippy::print_stderr,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(
    clippy::allow_attributes_without_reason,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]

use slopvac_pcre2::{Limits, MatchError, Options, Regex};

/// Targets where PCRE2's JIT is available and the build must have produced it.
const JIT_EXPECTED: bool = cfg!(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", target_os = "linux"),
    not(target_env = "musl")
));

#[allow(clippy::assertions_on_constants)]
/// A regex with confirmed JIT code, or `None` on a target without a JIT.
fn jit_regex(pattern: &str, limits: Limits) -> Option<Regex> {
    let re = Regex::with_limits(pattern, Options::new().jit(true), limits).unwrap();
    if re.is_jit() {
        return Some(re);
    }
    assert!(
        !JIT_EXPECTED,
        "this target must have a PCRE2 JIT, but {pattern:?} compiled without JIT code"
    );
    eprintln!("skipped: this target has no PCRE2 JIT");
    None
}

/// Few steps for the JIT at 12 `a`s, so a budget of 100 is spent and 100,000 is not.
const SPLITS: &str = r"(a|aa)+$";

fn splits_subject() -> String {
    format!("{}b", "a".repeat(12))
}

fn tight_interpreter(jit_match_limit: u32) -> Limits {
    Limits {
        match_limit: 1,
        jit_match_limit,
        ..Limits::POLICY
    }
}

#[test]
fn a_jit_budget_that_is_enough_finishes_on_the_jit() {
    // The interpreter alone, at limit 1, fails on this input; only the JIT can answer.
    let Some(re) = jit_regex(SPLITS, tight_interpreter(100_000)) else {
        return;
    };
    assert_eq!(re.is_match(&splits_subject()), Ok(false));
}

#[test]
fn the_default_jit_budget_is_honoured_too() {
    let limits = Limits {
        match_limit: 1,
        ..Limits::POLICY
    };
    let Some(re) = jit_regex(SPLITS, limits) else {
        return;
    };
    assert_eq!(re.is_match(&splits_subject()), Ok(false));
}

#[test]
fn a_spent_jit_budget_reaches_the_tighter_interpreter_limit() {
    let Some(re) = jit_regex(SPLITS, tight_interpreter(100)) else {
        return;
    };
    assert_eq!(
        re.is_match(&splits_subject()),
        Err(MatchError::MatchLimit { limit: 1 })
    );
}

#[test]
fn a_spent_jit_budget_is_retried_and_the_interpreter_may_succeed() {
    // Same spent budget, an interpreter that can afford the work: the retry answers.
    let limits = Limits {
        jit_match_limit: 100,
        ..Limits::POLICY
    };
    let Some(re) = jit_regex(SPLITS, limits) else {
        return;
    };
    assert_eq!(re.is_match(&splits_subject()), Ok(false));
}

const STACK_PATTERN: &str = r"^(?:a|b)*$";

fn stack_subject() -> String {
    "ab".repeat(50_000)
}

#[test]
fn a_roomy_jit_stack_finishes_on_the_jit() {
    let limits = Limits {
        match_limit: 1,
        ..Limits::POLICY
    };
    let Some(re) = jit_regex(STACK_PATTERN, limits) else {
        return;
    };
    assert_eq!(re.is_match(&stack_subject()), Ok(true));
}

#[test]
fn an_exhausted_jit_stack_reaches_the_tighter_interpreter_limit() {
    let limits = Limits {
        match_limit: 1,
        jit_stack_bytes: 1024,
        ..Limits::POLICY
    };
    let Some(re) = jit_regex(STACK_PATTERN, limits) else {
        return;
    };
    assert_eq!(
        re.is_match(&stack_subject()),
        Err(MatchError::MatchLimit { limit: 1 })
    );
}

#[test]
fn an_exhausted_jit_stack_is_retried_and_the_interpreter_may_succeed() {
    let limits = Limits {
        jit_stack_bytes: 1024,
        ..Limits::POLICY
    };
    let Some(re) = jit_regex(STACK_PATTERN, limits) else {
        return;
    };
    assert_eq!(re.is_match(&stack_subject()), Ok(true));
}

#[test]
fn the_jit_and_the_interpreter_agree_after_a_retry() {
    let spent = Limits {
        jit_match_limit: 1,
        ..Limits::POLICY
    };
    let subject = "one two  three\nfour";
    let pattern = r"\b\w+\b";
    let Some(retried) = jit_regex(pattern, spent) else {
        return;
    };
    let plain = Regex::new(pattern, Options::new()).unwrap();
    assert_eq!(
        retried.find_all(subject).unwrap(),
        plain.find_all(subject).unwrap()
    );
}
