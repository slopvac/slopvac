//! Context lookups on a wrapped table cell are bounded by a budget, so a pack cannot make a
//! document cost kinds times matches.
//!
//! A left cell wraps over many lines. Many taggers of distinct kinds each tag the whole
//! cell, and the engine splits each tag over the cell's lines. Rules that match across the
//! line break then look up the tags that cover a match that spans two lines, and each such
//! lookup looks at every kind on the cell, even when the context gate leaves no finding. The
//! tests count the looks, not the time.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![cfg(unix)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

mod common;

use common::{wrapped_rst, wrapped_table};
use serde_json::json;
use slopvac_context::{TagRegistry, TaggerSpec, take_lookup_steps};
use slopvac_engine::{Engine, EngineError, Input};

/// `kinds` taggers, each tagging a table cell that holds `Must` with a kind of its own.
fn taggers(kinds: usize) -> TagRegistry {
    let mut registry = TagRegistry::new();
    for i in 0..kinds {
        let yaml = format!(
            "schema_version: 3\nid: t{i}\ntag: k{i}\nunit: table-cell\nmatch:\n  - pattern: 'Must'\n"
        );
        let tagger = TaggerSpec::from_yaml(&yaml).unwrap().compile().unwrap();
        registry.register(Box::new(tagger)).unwrap();
    }
    registry
}

/// `rules` document-scope rules that each match the break between two lines of the cell, with
/// the context gate `gate`. A gate on `k0`, a kind the cell has, makes each lookup reach a real
/// kind.
fn engine(kinds: usize, rules: usize, lines: usize, gate: &serde_json::Value) -> Engine {
    let rules: Vec<_> = (0..rules)
        .map(|i| {
            common::rule(
                &format!("r{i}"),
                &json!({"kind": "pattern", "pattern": r"go\.\s+Must"}),
                &json!({"scope": "document", "contexts": gate}),
            )
        })
        .collect();
    common::engine(&rules)
        .with_rst(wrapped_rst(lines))
        .with_taggers(taggers(kinds))
}

/// The looks one run makes, and whether it finished. The gate skips `k0`, so the rules have no
/// findings.
fn run(kinds: usize, rules: usize, lines: usize) -> (usize, Result<usize, EngineError>) {
    run_gated(kinds, rules, lines, &json!({"skip": ["k0"]}))
}

fn run_gated(
    kinds: usize,
    rules: usize,
    lines: usize,
    gate: &serde_json::Value,
) -> (usize, Result<usize, EngineError>) {
    let engine = engine(kinds, rules, lines, gate);
    let source = wrapped_table(lines);
    let _ = take_lookup_steps();
    let result = engine
        .lint(&Input::rst("t.rst", &source))
        .map(|doc| doc.findings.len());
    (take_lookup_steps(), result)
}

#[test]
fn a_document_with_a_few_kinds_on_a_wrapped_cell_is_checked() {
    let (steps, result) = run(3, 4, 60);
    assert_eq!(result, Ok(0));
    // 60 lines give 59 breaks for each of 4 rules, and each lookup looks at the 3 kinds. All
    // the steps of the run, tree walks included, are under the looks the budget allows.
    assert!(steps > 0, "no steps counted");
    let limit = 8 * wrapped_table(60).len() + 16_384;
    assert!(steps < limit, "{steps} steps for a budget of {limit} looks");
}

#[test]
fn many_kinds_on_a_wrapped_cell_with_rules_across_its_lines_are_checked_in_linear_work() {
    let (kinds, rules, lines) = (400, 40, 60);
    let (steps, result) = run(kinds, rules, lines);
    // Each gate names `k0` alone, so a lookup asks about that kind and the other 399 kinds
    // on the cell cost it nothing. Looking at every kind took over nine
    // hundred thousand looks and stopped at the budget.
    assert_eq!(result, Ok(0));
    // Building the indexes is linear in the 24,000 pieces (about 66,000 steps, three
    // structures each), and the 2,360 lookups take a few steps each. The old code stopped
    // at the budget after about 270,000 steps, so this cap sits well under that.
    let cap = 100_000;
    assert!(steps < cap, "{steps} steps for a cap of {cap}");
}

#[test]
fn a_kind_that_both_gate_lists_name_is_looked_up_once() {
    // One tagger and 400 rules whose gates name `k0` in `only` and in `skip`. Each lookup
    // visits the cell's piece of `k0` once: 400 rules x 59 breaks, about 23,600 visits, under
    // the budget of 26,800. A lookup per list would visit twice and pass the budget.
    let gate = json!({"only": ["k0"], "skip": ["k0"]});
    let (_, result) = run_gated(1, 400, 60, &gate);
    assert_eq!(result, Ok(0));
}

#[test]
fn a_document_that_passes_the_lookup_budget_is_refused_with_its_budget_name_and_limit() {
    // 400 kinds on the cell, and 40 rules that each name every kind in the gate: each of the
    // 59 breaks looks at all 400 kinds for each rule, about 944,000 looks, against a budget of
    // 8 per byte of the document plus 16,384.
    let (kinds, rules, lines) = (400, 40, 60);
    let every_kind: Vec<String> = (0..kinds).map(|i| format!("k{i}")).collect();
    let (_, result) = run_gated(kinds, rules, lines, &json!({"only": every_kind}));
    let limit = 8 * wrapped_table(lines).len() + 16_384;
    let error = result.expect_err("the lookups pass their budget");
    assert_eq!(error, EngineError::TagLookupBudget { limit });
    assert_eq!(error.budget(), Some(("tag-lookups", limit)));
}
