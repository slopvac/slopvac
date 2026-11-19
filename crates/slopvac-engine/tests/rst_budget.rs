//! A wide RST grid table that exhausts the placement budget keeps its cells at line
//! precision, and the tagging of those cells neither panics nor reads another cell's text.
//!
//! The interpreter is a shell script that prints driver output the test writes. Every cell of
//! the row then shares the span of the row, and the cells differ in length and in multibyte
//! text, so a cell mapped through another cell's projection would have offsets off its text.
#![cfg(unix)]
#![allow(missing_docs)]

mod common;

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use common::{engine, tokens};
use serde_json::json;
use slopvac_engine::rst::RstOptions;
use slopvac_engine::{Engine, Input};

const COLUMNS: usize = 4_000;

fn wide_table() -> String {
    let border = format!("{}+\n", "+-".repeat(COLUMNS));
    let row = format!("{}|\n", "|a".repeat(COLUMNS));
    format!("{border}{row}{border}")
}

/// The driver output for the table: a cell per column, with text of a different length and
/// multibyte characters in each.
fn driver_output() -> String {
    let mut nodes =
        String::from(r#"{"tag":"table","table":true,"start":1,"end":3,"containers":[]}"#);
    for column in 0..COLUMNS {
        // The last cell is the short one that a lookup by span would find for every cell.
        let text = if column + 1 == COLUMNS {
            "a".to_owned()
        } else {
            "é. ".repeat(1 + column % 5)
        };
        write!(
            nodes,
            r#",{{"tag":"paragraph","start":2,"end":2,"containers":[],"cell":[0,{column},1],"pieces":[{{"k":"t","t":"{}"}}]}}"#,
            text.trim_end()
        )
        .unwrap();
    }
    format!(r#"{{"docutils":"0.21.2","nodes":[{nodes}]}}"#)
}

fn wide_engine() -> Engine {
    let dir = common::private_dir("rst-budget");
    let json = dir.join("wide.json");
    fs::write(&json, driver_output()).unwrap();
    let python = dir.join("wide-python");
    fs::write(
        &python,
        format!("#!/bin/sh\ncat >/dev/null\ncat '{}'\n", json.display()),
    )
    .unwrap();
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
    let mut options = RstOptions::default();
    options.interpreter = Some(python);
    engine(&[tokens(
        "dull",
        "é",
        &json!({"contexts": {"skip": ["requirement"]}}),
    )])
    .with_rst(options)
}

#[test]
fn a_table_that_exhausts_the_budget_lints_without_panicking() {
    let source = wide_table();
    let doc = wide_engine()
        .lint(&Input::rst("wide.rst", &source))
        .expect("the table lints");
    assert!(
        doc.unchecked
            .iter()
            .any(|n| n.code.as_str() == "rst-line-precise"),
        "the budget was not exhausted"
    );
    assert!(!doc.findings.is_empty(), "the cells' text was not checked");
}
