//! The per-finding context lookup through `Engine::lint` does work that grows near-linearly
//! with the document, not with the number of taggable regions times the number of findings.
//!
//! Each section of the test documents has a heading that the shipped `safety-heading`
//! tagger tags as a `safety` section, and a fenced block. Each section yields findings, and
//! each finding looks up the context kinds at its span. A lookup that scans every tag makes
//! the work grow with the square of the document, so doubling the input would quadruple it.
//! An indexed lookup costs about the logarithm of the tag count per finding, so doubling
//! the input a little more than doubles the work.
//!
//! The tests count index steps with `take_lookup_steps`, not time, so a loaded runner
//! cannot make them fail.
#![allow(missing_docs)]

mod common;

use common::{engine, rule, tokens};
use rstest::rstest;
use serde_json::json;
use slopvac_context::take_lookup_steps;
use slopvac_engine::{Engine, Input};

/// Sections in the smallest document. Each size doubles the one before.
const SIZES: [usize; 4] = [250, 500, 1_000, 2_000];

/// How much the work may grow when the input doubles. Linear growth gives 2; the logarithm
/// of the tag count adds about 0.2 at these sizes (measured 2.21). A linear scan per lookup
/// gives 4.
const DOUBLING_BOUND: f64 = 2.4;

/// Lexical findings: three per section, each gated to `safety` sections.
fn lexical() -> (Engine, impl Fn(usize) -> String, usize) {
    let engine = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"contexts": {"only": ["safety"]}}),
    )]);
    let source = |sections: usize| {
        "## Safety notes\n\nWe leverage it, leverage that and leverage this.\n\n```text\ncode\n```\n\n"
            .repeat(sections)
    };
    (engine, source, 3)
}

/// Diagram findings: one per section, from a fenced Mermaid block, dropped in `example` sections.
fn diagram() -> (Engine, impl Fn(usize) -> String, usize) {
    let engine = engine(&[rule(
        "label-run-on",
        &json!({"kind": "structure", "check": "diagram-label-run-on"}),
        &json!({"contexts": {"skip": ["example"]}}),
    )]);
    let source = |sections: usize| {
        "## Safety notes\n\n```mermaid\nflowchart TD\n  A[This label has far too many words in it] --> B[End]\n```\n\n"
            .repeat(sections)
    };
    (engine, source, 1)
}

/// Lookup steps and findings for one lint of `source`.
fn run(engine: &Engine, source: &str) -> (usize, usize) {
    let _ = take_lookup_steps();
    let doc = engine.lint(&Input::markdown("a.md", source)).unwrap();
    (take_lookup_steps(), doc.findings.len())
}

#[rstest]
#[case::lexical_findings(lexical())]
#[case::diagram_findings(diagram())]
fn doubling_the_document_less_than_two_and_a_half_times_the_context_lookup_work(
    #[case] shape: (Engine, impl Fn(usize) -> String, usize),
) {
    let (engine, source, per_section) = shape;
    let mut previous: Option<usize> = None;
    for sections in SIZES {
        let (steps, findings) = run(&engine, &source(sections));
        assert_eq!(findings, per_section * sections, "{sections} sections");
        assert!(steps > 0, "{sections} sections: no steps counted");
        if let Some(before) = previous {
            #[expect(clippy::cast_precision_loss, reason = "step counts are far below 2^52")]
            let growth = steps as f64 / before as f64;
            assert!(
                growth <= DOUBLING_BOUND,
                "{sections} sections: {before} -> {steps} steps, x{growth:.2}, bound x{DOUBLING_BOUND}"
            );
        }
        previous = Some(steps);
    }
}
