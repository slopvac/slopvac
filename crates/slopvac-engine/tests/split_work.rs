//! Tags on a wrapped cell of an RST grid table are split over the cell's own lines, in
//! work proportional to the table.
//!
//! The interpreter is a shell script that prints driver output the test writes: a left cell
//! that wraps over many lines, each a sentence, and a right cell beside the first line. The
//! two cells overlap in the source, so the tags of the left cell are split over its extents.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![cfg(unix)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

mod common;

use common::{engine, places, tokens, wrapped_rst, wrapped_table as table};
use serde_json::json;
use slopvac_engine::{Engine, Input, take_split_visits};

fn wrapped_engine(lines: usize) -> Engine {
    let gate = json!({"contexts": {"only": ["requirement"]}});
    engine(&[tokens("dull", "Must", &gate)]).with_rst(wrapped_rst(lines))
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

/// Steps per line of the wrapped cell that splitting the tags may take. The count is about two.
const STEPS_PER_LINE: usize = 8;

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
