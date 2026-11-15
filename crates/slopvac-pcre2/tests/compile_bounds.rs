//! Compile-time bounds: oversized, over-nested and over-large patterns are rejected with
//! typed errors instead of exhausting memory or the stack.

use slopvac_pcre2::{CompileError, MAX_PATTERN_BYTES, Options, PARENS_NEST_LIMIT, Regex};

#[test]
fn a_pattern_at_the_length_cap_compiles() {
    // A comment pads the pattern to the cap exactly: `a` + `(?#` + filler + `)`.
    let pattern = format!("a(?#{})", "x".repeat(MAX_PATTERN_BYTES - 5));
    assert_eq!(pattern.len(), MAX_PATTERN_BYTES);
    assert!(Regex::new(&pattern, Options::new()).is_ok());
}

#[test]
fn a_pattern_over_the_length_cap_is_rejected_with_its_size() {
    let pattern = "a".repeat(MAX_PATTERN_BYTES + 1);
    assert_eq!(
        Regex::new(&pattern, Options::new()).unwrap_err(),
        CompileError::TooLong {
            len: MAX_PATTERN_BYTES + 1,
            max: MAX_PATTERN_BYTES
        }
    );
}

#[test]
fn comment_padding_counts_toward_the_length_cap() {
    // PCRE2 allocates for the whole input before the comment disappears.
    let pattern = format!("a(?#{})", "x".repeat(MAX_PATTERN_BYTES));
    assert!(matches!(
        Regex::new(&pattern, Options::new()),
        Err(CompileError::TooLong { .. })
    ));
}

#[test]
fn nesting_at_the_limit_compiles_and_one_deeper_does_not() {
    let nest = |depth: u32| {
        let depth = depth as usize;
        format!("{}a{}", "(?:".repeat(depth), ")".repeat(depth))
    };
    assert!(Regex::new(&nest(PARENS_NEST_LIMIT), Options::new()).is_ok());
    let err = Regex::new(&nest(PARENS_NEST_LIMIT + 1), Options::new()).unwrap_err();
    assert!(
        matches!(
            err,
            CompileError::TooDeeplyNested {
                limit: PARENS_NEST_LIMIT,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_hugely_nested_pattern_is_an_error_not_a_stack_overflow() {
    let pattern = format!("{}a{}", "(".repeat(30_000), ")".repeat(30_000));
    assert!(matches!(
        Regex::new(&pattern, Options::new()),
        Err(CompileError::TooDeeplyNested { .. })
    ));
}

#[test]
fn a_pattern_whose_compiled_form_would_be_enormous_is_rejected() {
    // 255^3 repetitions of one character: far past PCRE2's compiled-size ceiling.
    let result = Regex::new(r"(((a{255}){255}){255}){255}", Options::new());
    assert!(
        matches!(result, Err(CompileError::Syntax { .. })),
        "{result:?}"
    );
}

#[test]
fn a_variable_length_lookbehind_branch_over_the_limit_is_rejected() {
    // 256 characters exceed the 255 limit; 255 are allowed.
    let within = Regex::new(r"(?<=a{1,255})b", Options::new());
    let beyond = Regex::new(r"(?<=a{1,256})b", Options::new());
    assert!(within.is_ok(), "{within:?}");
    assert!(
        matches!(beyond, Err(CompileError::Syntax { .. })),
        "{beyond:?}"
    );
}
