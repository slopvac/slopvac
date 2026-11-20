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
    let path = vec!["k"; 70].join(".");
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
fn migrate_config_refuses_with_a_located_parse_error(#[case] body: String) {
    let text = format!("profile = \"normal\"\n{body}");
    assert!(text.len() < slopvac_config::MAX_CONFIG_BYTES as usize);
    let error = refused(&text);
    assert_eq!(error.code(), "config-invalid-toml");
    assert_eq!(
        error.to_string(),
        "slopvac.toml: line 2: tables nest more than 32 levels deep"
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
    assert_eq!(error.code(), "config-invalid-toml");
    assert!(
        error
            .to_string()
            .ends_with("line 5: tables nest more than 32 levels deep"),
        "{error}"
    );
}
