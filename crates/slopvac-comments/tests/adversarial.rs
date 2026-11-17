//! Hostile input: no panics, bounded work.

mod common;

use common::fragments;
use slopvac_comments::{ExtractError, Language, Limits, extract, extract_lexical, extract_with};

/// Work for `n` and `8 * n` units. Linear work grows about 8x. A quadratic pass grows
/// about 64x. Steps are counted, not timed, so the result is the same on every machine.
fn assert_near_linear(language: Language, make: impl Fn(usize) -> String) {
    let steps = |n: usize| {
        extract_with(language, &make(n), &Limits::default())
            .expect("extracts")
            .stats
            .walk_steps
    };
    let (small, large) = (steps(2_000), steps(16_000));
    assert!(
        large <= small * 10,
        "{language:?}: {small} steps for n, {large} for 8n"
    );
}

#[test]
fn many_line_comments_scale_linearly() {
    assert_near_linear(Language::Rust, |n| "// word word\n".repeat(n));
}

#[test]
fn many_comments_on_one_line_scale_linearly() {
    assert_near_linear(Language::C, |n| "/* a */ ".repeat(n));
}

#[test]
fn a_long_run_of_blanks_before_each_comment_scales_linearly() {
    assert_near_linear(Language::Python, |n| format!("{}# x\n", " ".repeat(n)));
}

#[test]
fn many_docstring_candidates_scale_linearly() {
    assert_near_linear(Language::Python, |n| "\"\"\"x\"\"\"\n".repeat(n));
}

#[test]
fn a_huge_comment_extracts() {
    let source = format!("// {}\n", "word ".repeat(2_000_000));
    let fragments = fragments(Language::Rust, &source);
    assert_eq!(fragments.len(), 1);
    assert_eq!(fragments[0].text().len(), source.len() - "// \n".len() - 1);
}

#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    let source = format!("// top\n{}\n", "(".repeat(200_000));
    for language in [Language::Rust, Language::JavaScript, Language::Python] {
        let _ = extract(language, &source).expect("extracts");
    }
}

#[test]
fn unterminated_comments_and_strings_do_not_panic() {
    let sources = [
        "/* never closed\nstill open",
        "/**",
        "/*",
        "/*/",
        "/**/",
        "\"\"\"unterminated docstring\n",
        "'''",
        "\"",
        "--[==[ open",
        "=begin\nopen",
        "{- open",
        "<# open",
        "#=",
        "(* open",
        "// ",
        "//",
        "#",
        "\r",
        "\r\n\r\n",
    ];
    for language in Language::all() {
        for source in sources {
            let lexed = extract_lexical(language, source, &Limits::default()).expect("lexes");
            let result = extract(language, source).expect("extracts");
            for fragment in lexed.fragments.iter().chain(&result) {
                let span = fragment.comment_span();
                assert!(span.end() <= source.len());
                assert!(source.is_char_boundary(span.start()));
                assert!(source.is_char_boundary(span.end()));
            }
        }
    }
}

#[test]
fn invalid_syntax_still_yields_the_comments_it_can_see() {
    let got = fragments(Language::Rust, "fn ((( {{{ // kept\n@@@ ### \n// also\n");
    assert!(got.iter().any(|f| f.text().contains("kept")));
}

#[test]
fn binary_looking_text_does_not_panic() {
    let noise: String = (0u32..4000)
        .map(|i| char::from_u32(0x20 + (i * 7919) % 0x2000).unwrap_or('?'))
        .collect();
    for language in Language::all() {
        let wrapped = format!("// {noise}\n/* {noise} */\n# {noise}\n");
        for source in [&noise, &wrapped] {
            // The Scala and F# scanners may stop at their read budget; nothing else may fail.
            match extract(language, source) {
                Ok(_) | Err(ExtractError::ReadBudget { .. } | ExtractError::ParseBudget { .. }) => {
                }
                Err(error) => panic!("{language:?}: {error}"),
            }
        }
    }
}

/// The lexer reads the source in 4 KiB chunks. A multi-byte character that straddles a
/// chunk edge once crashed the Dart scanner, so every language reads one at each offset.
#[test]
fn multi_byte_characters_at_every_chunk_edge_do_not_crash() {
    for language in Language::all() {
        for pad in 4090..4097 {
            let source = format!(
                "// {}é€😀 end\n{}😀é€\n",
                "a".repeat(pad - 3),
                "b".repeat(pad)
            );
            match extract(language, &source) {
                Ok(_) | Err(ExtractError::ReadBudget { .. } | ExtractError::ParseBudget { .. }) => {
                }
                Err(error) => panic!("{language:?} pad {pad}: {error}"),
            }
        }
    }
}
/// Every language, through the lexer, on input built to confuse a comment scanner.
#[test]
fn the_lexer_survives_confusing_input_in_every_language() {
    let sources = [
        "\"\"\"unterminated\n# not a comment",
        "'''unterminated",
        "'\\",
        "'",
        "\"\\",
        "`unterminated\n// x",
        "/* /* /* nested",
        "--[==[ a ]=] b",
        "=begin\n=end",
        "=begin\nnever ends",
        "#=",
        "{- {- -}",
        "(* (* *)",
        "// a \\",
        "// a \\\r\n// b",
        "\u{feff}#!shebang\n",
        "#![attr]",
        "\u{1F600}'\u{1F600}'",
        "'\\u{1F600}'",
    ];
    for language in Language::all() {
        for source in sources {
            let extraction = extract_lexical(language, source, &Limits::default()).expect("lexes");
            for fragment in &extraction.fragments {
                let span = fragment.comment_span();
                assert!(
                    source.is_char_boundary(span.start()) && source.is_char_boundary(span.end())
                );
            }
        }
    }
}
/// The lexer stops at the step budget instead of scanning the whole input first.
#[test]
fn a_tiny_walk_budget_stops_the_lexer_early_on_a_large_input() {
    let source = "// word word word\n".repeat(200_000);
    let limits = Limits {
        max_walk_steps: Some(50),
        ..Limits::default()
    };
    let error = extract_lexical(Language::Rust, &source, &limits).expect_err("over budget");
    assert!(
        matches!(error, ExtractError::WalkBudget { budget: 50 }),
        "{error:?}"
    );
}
