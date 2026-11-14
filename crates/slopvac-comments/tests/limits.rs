#![cfg(feature = "all-languages")]
//! Work bounds: deterministic counters, not wall-clock time.

mod common;

use slopvac_comments::{ExtractError, Language, Limits, extract_with};

fn steps(language: Language, source: &str) -> usize {
    extract_with(language, source, &Limits::default())
        .expect("extracts")
        .stats
        .walk_steps
}

/// Work for an input of `n` and `4 * n` units. Linear work grows 4x; a quadratic pass 16x.
fn assert_walk_is_linear(language: Language, make: impl Fn(usize) -> String) {
    let small = steps(language, &make(2_000));
    let large = steps(language, &make(8_000));
    assert!(
        large <= small * 5,
        "{language:?}: {small} steps for n, {large} for 4n"
    );
}

#[test]
fn python_docstring_detection_is_linear_with_many_comments_and_strings() {
    assert_walk_is_linear(Language::Python, |n| {
        format!("{}{}", "# c\n".repeat(n), "'s'\n# c\n".repeat(n))
    });
}

#[test]
fn python_docstring_detection_is_linear_with_many_functions() {
    assert_walk_is_linear(Language::Python, |n| {
        "def f():\n    # c\n    '''d'''\n".repeat(n)
    });
}

#[test]
fn julia_docstring_detection_is_linear_with_many_strings() {
    assert_walk_is_linear(Language::Julia, |n| "# c\n\"doc\"\nx = 1\n".repeat(n));
}

#[test]
fn the_walk_visits_each_node_a_bounded_number_of_times() {
    assert_walk_is_linear(Language::Rust, |n| "fn f() { g(1, 2); } // c\n".repeat(n));
}

#[test]
fn a_parse_budget_that_is_too_small_returns_a_typed_error() {
    let source = "// a\nfn main() { let x = 1; let y = 2; }\n".repeat(200);
    let limits = Limits {
        max_parse_checks: Some(1),
        ..Limits::default()
    };
    let error = extract_with(Language::Rust, &source, &limits).expect_err("over budget");
    assert!(
        matches!(error, ExtractError::ParseBudget { budget: 1, .. }),
        "{error:?}"
    );
}

#[test]
fn a_walk_budget_that_is_too_small_returns_a_typed_error() {
    let limits = Limits {
        max_walk_steps: Some(3),
        ..Limits::default()
    };
    let error =
        extract_with(Language::Rust, "// a\nfn f() {}\n", &limits).expect_err("over budget");
    assert!(
        matches!(error, ExtractError::WalkBudget { budget: 3 }),
        "{error:?}"
    );
}

#[test]
fn a_source_over_the_size_limit_is_refused_before_parsing() {
    let limits = Limits {
        max_bytes: 10,
        ..Limits::default()
    };
    let error = extract_with(Language::Rust, "// 12345678\n", &limits).expect_err("too large");
    assert!(
        matches!(error, ExtractError::TooLarge { len: 12, max: 10 }),
        "{error:?}"
    );
}

/// Inputs that make some parsers backtrack or recover heavily. The default budget must
/// admit all of them, so a real file never fails for being unusual.
#[test]
fn awkward_input_stays_inside_the_default_budget() {
    let deep = "(".repeat(5_000);
    let noise = "}{)(][ @#$ ".repeat(2_000);
    let strings = "\"\"\"'''`".repeat(2_000);
    let mut over = Vec::new();
    for language in Language::all() {
        for (name, source) in [("deep", &deep), ("noise", &noise), ("strings", &strings)] {
            match extract_with(language, source, &Limits::default()) {
                Ok(_) => {}
                Err(error) => over.push(format!("{language:?} {name}: {error}")),
            }
        }
    }
    assert!(over.is_empty(), "{}", over.join("\n"));
}

/// A run of unterminated `/*` makes the C and Go parsers quadratic: 3 000 of them take
/// about 90 000 progress checks. The default budget turns that into an error.
#[test]
fn a_quadratic_parse_stops_at_the_default_budget() {
    let source = "/* ".repeat(3_000);
    for language in [Language::C, Language::Go] {
        let error = extract_with(language, &source, &Limits::default()).expect_err("over budget");
        assert!(
            matches!(error, ExtractError::ParseBudget { .. }),
            "{language:?}: {error:?}"
        );
    }
}
