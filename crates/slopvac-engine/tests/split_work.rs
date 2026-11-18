//! Tags on a wrapped cell of an RST grid table are split over the cell's own lines, in
//! work proportional to the table.
//!
//! The interpreter is a shell script that prints driver output the test writes: a left cell
//! that wraps over many lines, each a sentence, and a right cell beside the first line. The
//! two cells overlap in the source, so the tags of the left cell are split over its extents.
#![cfg(unix)]
#![allow(missing_docs)]

mod common;

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use common::{engine, places, tokens};
use serde_json::json;
use slopvac_engine::rst::RstOptions;
use slopvac_engine::{Engine, Input, take_split_visits};

/// The table: a left cell of `lines` lines that each read `Must go.`, and `x` beside the first.
fn table(lines: usize) -> String {
    let border = format!("+{}+{}+\n", "-".repeat(12), "-".repeat(5));
    let mut source = border.clone();
    for line in 0..lines {
        let right = if line == 0 { "x" } else { " " };
        writeln!(source, "| Must go.   | {right}   |").unwrap();
    }
    source.push_str(&border);
    source
}

/// The driver output for the table.
fn driver_output(lines: usize) -> String {
    let left = vec!["Must go."; lines].join("\\n");
    let cell = |end: usize, column: usize, text: &str| {
        format!(
            r#"{{"tag":"paragraph","start":2,"end":{end},"containers":[],"cell":[0,{column},1],"pieces":[{{"k":"t","t":"{text}"}}]}}"#
        )
    };
    format!(
        r#"{{"docutils":"0.21.2","nodes":[{{"tag":"table","table":true,"start":1,"end":{},"containers":[]}},{},{}]}}"#,
        lines + 2,
        cell(lines + 1, 0, &left),
        cell(2, 1, "x"),
    )
}

fn wrapped_engine(lines: usize) -> Engine {
    let dir = common::private_dir("rst-split");
    let json_path = dir.join(format!("wrapped-{lines}.json"));
    fs::write(&json_path, driver_output(lines)).unwrap();
    let python = dir.join(format!("wrapped-python-{lines}"));
    fs::write(
        &python,
        format!("#!/bin/sh\ncat >/dev/null\ncat '{}'\n", json_path.display()),
    )
    .unwrap();
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
    let mut options = RstOptions::default();
    options.interpreter = Some(python);
    let gate = json!({"contexts": {"only": ["requirement"]}});
    engine(&[tokens("dull", "Must", &gate)]).with_rst(options)
}

#[test]
fn each_line_of_a_wrapped_cell_is_checked_as_the_requirement_it_is() {
    let lines = 6;
    let doc = wrapped_engine(lines)
        .lint(&Input::rst("t.rst", &table(lines)))
        .unwrap();
    // The first word of the cell on each of its lines, after the border and the `| `.
    let want: Vec<_> = (0..lines)
        .map(|line| ("core:cat.dull".to_owned(), 2 + line, 3))
        .collect();
    assert_eq!(places(&doc), want);
}

/// Steps per line of the wrapped cell that splitting the tags may take. Each of the four
/// sentence taggers tags every line once, and each tag meets its own extent.
const STEPS_PER_LINE: usize = 24;

#[test]
fn splitting_the_tags_of_a_wrapped_cell_takes_steps_in_proportion_to_its_lines() {
    for lines in [200, 400, 800] {
        let engine = wrapped_engine(lines);
        let source = table(lines);
        let _ = take_split_visits();
        engine.lint(&Input::rst("t.rst", &source)).unwrap();
        let steps = take_split_visits();
        let limit = STEPS_PER_LINE * lines;
        assert!(
            steps > 0 && steps <= limit,
            "{lines} lines: {steps} steps, limit {limit}"
        );
    }
}
