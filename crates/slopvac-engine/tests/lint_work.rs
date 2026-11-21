//! The work the context taggers do through `Engine::lint` grows linearly with the document.
//!
//! The taggers count the blocks they examine for section bounds, the text positions they
//! clean, and the bytes of heading and admonition text they match. The test reads the counters
//! of this thread after a lint of documents that double in size, and bounds each by a multiple
//! of the document's size plus a constant. A quadratic path passes at the smallest size and
//! overshoots at the largest.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

mod common;

use common::engine;
use slopvac_context::{
    take_admonition_steps, take_heading_steps, take_section_steps, take_text_steps,
};
use slopvac_engine::Input;

/// A section with a heading, a paragraph of short sentences, a link whose tail is cleaned,
/// a quote and an admonition.
const SECTION: &str = "## Setup\n\nGo on. See [the guide](https://example.test/a/b) now. Stop.\n\n> Quoted. Text.\n\n!!! note \"Title\"\n    Body text.\n\n";

/// Steps per byte of source that the taggers may take. The measured counts are below 2.
const STEPS_PER_BYTE: usize = 8;

#[test]
fn a_lint_of_a_document_with_many_sections_takes_steps_in_proportion_to_its_size() {
    let engine = engine(&[]);
    for sections in [500, 1_000, 2_000, 4_000] {
        let source = format!("# Top\n\n{}", SECTION.repeat(sections));
        let _unused = (
            take_section_steps(),
            take_heading_steps(),
            take_text_steps(),
            take_admonition_steps(),
        );
        engine.lint(&Input::markdown("a.md", &source)).unwrap();
        let steps = [
            take_section_steps(),
            take_heading_steps(),
            take_text_steps(),
            take_admonition_steps(),
        ];
        let bound = STEPS_PER_BYTE * source.len() + 1_000;
        for (name, used) in ["section", "heading", "text", "admonition"]
            .iter()
            .zip(steps)
        {
            assert!(
                used <= bound,
                "{name}: {used} steps for {sections} sections ({} bytes), bound {bound}",
                source.len()
            );
        }
        // The counters run on this thread, so a lint that moved the taggers elsewhere would
        // pass the bound with zeros.
        assert!(steps[..3].iter().all(|&n| n > 0), "{steps:?}");
    }
}
