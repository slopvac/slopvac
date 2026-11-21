#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "test crate: nothing public to document")]
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
fn the_walk_visits_each_node_a_bounded_number_of_times() {
    assert_walk_is_linear(Language::Rust, |n| "fn f() { g(1, 2); } // c\n".repeat(n));
}

#[test]
fn a_parse_budget_that_is_too_small_returns_a_typed_error() {
    let source = "// a\nint main() { int x = 1; int y = 2; }\n".repeat(200);
    let limits = Limits {
        max_parse_checks: Some(1),
        ..Limits::default()
    };
    let error = extract_with(Language::C, &source, &limits).expect_err("over budget");
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

/// Inputs that make some parsers backtrack or recover heavily. Each must finish with
/// fragments or with a typed budget error, never another error. The Scala and F#
/// scanners rescan on garbage and on deep nesting, so they may hit the read budget.
#[test]
fn awkward_input_ends_in_fragments_or_a_typed_budget_error() {
    let deep = "(".repeat(5_000);
    let noise = "}{)(][ @#$ ".repeat(2_000);
    let strings = "\"\"\"'''`".repeat(2_000);
    let mut over = Vec::new();
    for language in Language::all() {
        for (name, source) in [("deep", &deep), ("noise", &noise), ("strings", &strings)] {
            match extract_with(language, source, &Limits::default()) {
                Ok(_)
                | Err(
                    ExtractError::ParseBudget { .. }
                    | ExtractError::ReadBudget { .. }
                    | ExtractError::WalkBudget { .. },
                ) => {}
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
            matches!(
                error,
                ExtractError::ParseBudget { .. } | ExtractError::ReadBudget { .. }
            ),
            "{language:?}: {error:?}"
        );
    }
}
#[test]
fn padded_unterminated_comment_openers_hit_the_read_budget_in_c_and_go() {
    // Each unmatched `/*` makes these lexers scan to the end of the file, so the reads
    // grow with openers x padding while the parser action count stays small.
    let source = format!("{}{}\n", "/* ".repeat(2_000), "x".repeat(400_000));
    for language in [Language::C, Language::Go] {
        let error = extract_with(language, &source, &Limits::default()).expect_err("over budget");
        assert!(
            matches!(error, ExtractError::ReadBudget { .. }),
            "{language:?}: {error:?}"
        );
    }
}

#[test]
fn a_read_budget_that_is_too_small_returns_a_typed_error() {
    let limits = Limits {
        max_read_bytes: Some(10),
        ..Limits::default()
    };
    let error = extract_with(Language::C, "// a\nfn f() {}\n", &limits).expect_err("over budget");
    assert!(
        matches!(error, ExtractError::ReadBudget { budget: 10, .. }),
        "{error:?}"
    );
}

/// Real code is read about once. This is a C-style file of ordinary code, comments and
/// strings; the budget is 64 reads per byte.
#[test]
fn realistic_files_read_each_byte_about_once() {
    let block = "/* Compute the total. */\nint total(int a, int b) { // add\n    char *s = \"x // y\";\n    return a + b;\n}\n";
    let source = block.repeat(2_000);
    for language in [Language::C, Language::Go, Language::Java] {
        let stats = extract_with(language, &source, &Limits::default())
            .expect("extracts")
            .stats;
        println!(
            "READ {language:?} {} of {} bytes",
            stats.read_bytes,
            source.len()
        );
        assert!(
            stats.read_bytes < 4 * source.len(),
            "{language:?}: read {} bytes for {}",
            stats.read_bytes,
            source.len()
        );
    }
}

/// The lexer does one pass, so even the input that makes the C and Go parsers quadratic
/// costs one step per byte or so.
#[test]
fn the_lexer_is_linear_on_input_that_is_quadratic_for_tree_sitter() {
    let steps = |n: usize| {
        let source = format!("{}{}\n", "/* ".repeat(n), "x".repeat(100 * n));
        slopvac_comments::extract_lexical(Language::C, &source, &Limits::default())
            .expect("lexes")
            .stats
            .walk_steps
    };
    let (small, large) = (steps(2_000), steps(8_000));
    assert!(large <= small * 5, "{small} steps for n, {large} for 4n");
}

#[test]
fn the_lexer_is_linear_on_many_strings_and_comments() {
    for language in Language::all() {
        let steps = |n: usize| {
            slopvac_comments::extract_lexical(
                language,
                &"x = \"a\" # b // c /* d */ 'e' `f`\n".repeat(n),
                &Limits::default(),
            )
            .expect("lexes")
            .stats
            .walk_steps
        };
        let (small, large) = (steps(1_000), steps(4_000));
        assert!(
            large <= small * 5,
            "{language:?}: {small} steps for n, {large} for 4n"
        );
    }
}
