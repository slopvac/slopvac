//! Finding the paragraphs near each diagram does work in proportion to the document.
//!
//! The test counts block visits rather than timing the lint, so a loaded runner cannot make
//! it fail. It is the only test in this binary, so no other lint adds to the counter.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

mod common;

use common::engine;
use slopvac_engine::{
    DiagramChecker, DiagramHit, DiagramInput, DiagramRule, Input, nearby_block_visits,
};

/// Accepts every diagram and reports nothing.
struct Quiet;

impl DiagramChecker for Quiet {
    fn rules(&self) -> Vec<DiagramRule> {
        Vec::new()
    }

    fn check(&self, _input: &DiagramInput<'_>) -> Vec<DiagramHit> {
        Vec::new()
    }
}

const FENCE: &str = "```mermaid\nflowchart TD\n```\n";

/// A document and its number of top-level blocks.
struct Shape {
    name: &'static str,
    source: fn(usize) -> String,
    blocks: fn(usize) -> usize,
}

const SHAPES: [Shape; 3] = [
    // No paragraph anywhere: every cursor runs to the end of the blocks.
    Shape {
        name: "no paragraph",
        source: |n| FENCE.repeat(n),
        blocks: |n| n,
    },
    Shape {
        name: "one paragraph either side",
        source: |n| format!("Before.\n\n{}\nAfter.\n", FENCE.repeat(n)),
        blocks: |n| n + 2,
    },
    Shape {
        name: "paragraph between fences",
        source: |n| "Prose.\n\n".to_owned() + &format!("{FENCE}\nProse.\n\n").repeat(n),
        blocks: |n| 2 * n + 1,
    },
];

/// Visits per block and fence that the lookup may make. The lookup walks the blocks a small
/// fixed number of times; a per-fence rescan would exceed this at a thousand fences.
const VISITS_PER_BLOCK_AND_FENCE: usize = 4;

#[test]
fn finding_nearby_prose_visits_each_block_a_bounded_number_of_times() {
    let engine = engine(&[]).with_diagram_checker(Box::new(Quiet));
    for shape in &SHAPES {
        for fences in [1_000, 2_000, 4_000] {
            let source = (shape.source)(fences);
            let before = nearby_block_visits();
            engine.lint(&Input::markdown("a.md", &source)).unwrap();
            let visits = nearby_block_visits() - before;
            let limit = VISITS_PER_BLOCK_AND_FENCE * ((shape.blocks)(fences) + fences);
            assert!(
                visits <= limit,
                "{}, {fences} fences: {visits} block visits, limit {limit}",
                shape.name
            );
        }
    }
}
