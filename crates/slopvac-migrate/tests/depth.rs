//! A deeply nested hostile config ends in a located error, never a stack overflow.
//!
//! Every input is under `MAX_CONFIG_BYTES`. The parser allows 79 levels of inline tables, 79
//! parts in one dotted key or header, and 79 in each key of each nested inline table, so
//! these shapes sit inside its limits but far past what the recursive walks can take.

use std::fs;

use rstest::rstest;
use slopvac_migrate::{Error, Options, RenameTable, migrate_config, plan};

const STEERING: &str = "<!-- slopvac:begin -->\nnew\n<!-- slopvac:end -->\n";

fn inline(levels: usize) -> String {
    format!("a = {}1{}\n", "{a = ".repeat(levels), "}".repeat(levels))
}

fn dotted(levels: usize) -> String {
    format!("{} = 1\n", vec!["a"; levels + 1].join("."))
}

fn header(levels: usize) -> String {
    format!("[{}]\n", vec!["a"; levels].join("."))
}

/// 70 dotted parts inside each of 70 nested inline tables: about 4900 tables.
fn dotted_in_inline(levels: usize) -> String {
    dotted_in_inline_parts(70, levels)
}

/// `parts` dotted parts inside each of `levels` nested inline tables.
fn dotted_in_inline_parts(parts: usize, levels: usize) -> String {
    let path = vec!["k"; parts].join(".");
    format!(
        "{}x = 1{}\n",
        format!("{path} = {{ ").repeat(levels),
        " }".repeat(levels)
    )
}

fn refused(text: &str) -> Error {
    migrate_config(
        text,
        "slopvac.toml",
        &RenameTable::bundled(),
        Options::default(),
    )
    .expect_err("a config this deep is refused")
}

#[rstest]
#[case::inline_tables(inline(70))]
#[case::dotted_keys(dotted(70))]
#[case::headers(header(70))]
#[case::dotted_keys_in_inline_tables(dotted_in_inline(70))]
fn migrate_config_refuses_with_a_located_dropped_record(#[case] body: String) {
    let text = format!("profile = \"normal\"\n{body}");
    assert!(u64::try_from(text.len()).unwrap() < slopvac_config::MAX_CONFIG_BYTES);
    let error = refused(&text);
    assert_eq!(error.code(), "config-legacy-dropped");
    assert_eq!(
        error.to_string(),
        "slopvac.toml:2: `tables`: tables nest more than 32 levels deep"
    );
}

#[test]
fn plan_refuses_a_deep_pyproject_table_with_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let text = format!(
        "[project]\nname = \"x\"\n\n[tool.slopvac]\n{}",
        dotted_in_inline(70)
    );
    fs::write(dir.path().join("pyproject.toml"), text).unwrap();
    let error = plan(
        dir.path(),
        &RenameTable::bundled(),
        Options::default(),
        STEERING,
    )
    .expect_err("a config this deep is refused");
    assert_eq!(error.code(), "config-legacy-dropped");
    assert_eq!(
        error.to_string(),
        "pyproject.toml:5: `tables`: tables nest more than 32 levels deep"
    );
}

/// The deepest tree the parser accepts, 80 dotted parts in each of 80 nested inline tables,
/// is refused from a thread with a small stack. Recursion over a tree that deep, to build,
/// walk or drop it, needs far more than this stack on any target and in any build profile.
#[test]
fn the_deepest_tree_the_parser_accepts_is_refused_from_a_small_stack() {
    let text = format!("profile = \"normal\"\n{}", dotted_in_inline_parts(80, 80));
    let error = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || refused(&text))
        .expect("the test thread starts")
        .join()
        .expect("the refusal returns instead of panicking");
    assert_eq!(error.code(), "config-legacy-dropped");
    assert_eq!(
        error.to_string(),
        "slopvac.toml:2: `tables`: tables nest more than 32 levels deep"
    );
}

/// A deep valid prefix followed by a syntax error must still end in a located parse error
/// from a small stack: the failed parse may have built the deep partial tree first.
#[rstest]
#[case::duplicate_root_key("profile = \"strict\"\n")]
#[case::trailing_garbage("= = =\n")]
fn a_deep_prefix_before_a_syntax_error_is_a_parse_error_from_a_small_stack(#[case] tail: &str) {
    let text = format!(
        "profile = \"normal\"\n{}{tail}",
        dotted_in_inline_parts(80, 80)
    );
    let error = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || refused(&text))
        .expect("the test thread starts")
        .join()
        .expect("the refusal returns instead of panicking");
    assert_eq!(error.code(), "config-parse");
}
