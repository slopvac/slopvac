//! Each budget a document can pass has its own refusal text: the text names the budget that
//! ran out and its limit, and holds none of the document.
#![cfg(unix)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

mod common;

use rstest::rstest;
use serde_json::json;
use slopvac_engine::{Engine, Input};

fn engine() -> Engine {
    common::engine(&[common::tokens("r", "leverage", &json!({}))])
}

/// A table of `rows` rows of `columns` cells of one character, with its header.
fn table(rows: usize, columns: usize) -> String {
    let row = format!("{}|\n", "|a".repeat(columns));
    let rule = format!("{}|\n", "|-".repeat(columns));
    format!("{row}{rule}{}", row.repeat(rows))
}

/// 150000 requirements of 8 bytes: a tag for every 9 bytes, over the one for every 64.
fn tags() -> (String, usize) {
    let source = format!("{}\n", vec!["It must."; 150_000].join(" "));
    let limit = source.len() / 64 + 65_536;
    (source, limit)
}

/// 300000 sentences of 3 bytes: over one sentence for every 8 bytes plus 65,536.
fn sentences() -> (String, usize) {
    let source = format!("{}\n", vec!["Go."; 300_000].join(" "));
    let limit = source.len() / 8 + 65_536;
    (source, limit)
}

/// 300000 cells of one character: over one cell for every 4 bytes plus 65,536.
fn table_cells() -> (String, usize) {
    let source = table(3_000, 100);
    let limit = source.len() / 4 + 65_536;
    (source, limit)
}

#[rstest]
#[case::tags(tags(), "context tags")]
#[case::sentences(sentences(), "sentences")]
#[case::table_cells(table_cells(), "table cells")]
fn a_refusal_names_its_budget_and_limit_and_none_of_the_document(
    #[case] input: (String, usize),
    #[case] noun: &str,
) {
    let (source, limit) = input;
    let error = engine()
        .lint(&Input::markdown("a.md", &source))
        .map(|d| d.findings.len())
        .expect_err("the document passes a budget");
    let message = error.to_string();
    let expected = format!("the document has more than {limit} {noun}");
    assert!(message.starts_with(&expected), "{message}");
    assert!(
        !message.contains("must") && !message.contains("Go."),
        "{message}"
    );
}
