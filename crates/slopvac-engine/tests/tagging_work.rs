//! Tagging a paragraph of short sentences does work in proportion to its size.
//!
//! The test counts the units of projection scanned to map sentences to source rather than
//! timing the lint, so a loaded runner cannot make it fail. It is the only test in this
//! binary, so no other lint adds to the counter.

mod common;

use common::engine;
use slopvac_engine::Input;
use slopvac_parse::scan_work;

/// Units of work per sentence that the mapping may do. Each of the default sentence taggers maps
/// the sentences of a block once, and each lookup costs one unit here.
const WORK_PER_SENTENCE: usize = 8;

#[test]
fn mapping_the_sentences_of_a_paragraph_costs_a_bounded_amount_of_work_each() {
    let engine = engine(&[]);
    for sentences in [1_000, 2_000, 4_000, 8_000] {
        let source = "Go on. ".repeat(sentences);
        let before = scan_work();
        engine.lint(&Input::markdown("a.md", &source)).unwrap();
        let work = scan_work() - before;
        let limit = WORK_PER_SENTENCE * sentences;
        assert!(
            work <= limit,
            "{sentences} sentences: {work} units of work, limit {limit}"
        );
    }
}
