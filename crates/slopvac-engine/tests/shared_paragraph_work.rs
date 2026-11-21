//! One long paragraph beside many adjacent fences is analysed once and reported once.
//!
//! The tests count paragraph analyses (`test-counters`) and retained bytes rather than timing
//! the lint, so a loaded runner cannot make them fail. The counter is process-wide, so the
//! tests take a lock and read a difference.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

mod common;

use std::sync::{Mutex, MutexGuard};

use common::{category, ruleset, settings};
use slopvac_diagrams::prose_analyses;
use slopvac_engine::{DIAGRAM_PACK, Engine, Input, diagram_rule_set, prose_finding_bytes};
use slopvac_model::{Finding, PackId};

static SERIAL: Mutex<()> = Mutex::new(());

const FENCES: usize = 200;

const LABELS: &str = "flowchart TD\n  A[Load Balancer] --> B[API Gateway]\n  B --> C[Auth Service]\n  C --> D[User Database]\n";

/// A paragraph of more than 40 words that names every label and points at the diagram.
const PARAGRAPH: &str = "As shown in the diagram below, requests reach the load balancer first. The balancer forwards each request to the API gateway. The gateway asks the auth service to check the token, and then the user database returns the account record for that user. Responses travel back through the same path to the client over the network connection.";

fn engine() -> Engine {
    let pack = diagram_rule_set(&PackId::new(DIAGRAM_PACK).unwrap()).unwrap();
    Engine::new(vec![ruleset(vec![category("cat", &[])]), pack], settings()).unwrap()
}

/// One paragraph, then `FENCES` adjacent fences: the paragraph is the prose before each.
fn document() -> String {
    format!(
        "{PARAGRAPH}\n\n{}",
        format!("```mermaid\n{LABELS}```\n\n").repeat(FENCES)
    )
}

/// The text of the one meta reference in the paragraph.
const META: &str = "As shown in the diagram";

struct Run {
    analyses: usize,
    built_bytes: usize,
    findings: Vec<Finding>,
}

fn run() -> (MutexGuard<'static, ()>, Run) {
    let guard = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let source = document();
    let (before, built_before) = (prose_analyses(), prose_finding_bytes());
    let findings = engine()
        .lint(&Input::markdown("a.md", &source))
        .unwrap()
        .findings;
    let analyses = prose_analyses() - before;
    let built_bytes = prose_finding_bytes() - built_before;
    (
        guard,
        Run {
            analyses,
            built_bytes,
            findings,
        },
    )
}

fn of_rule<'a>(run: &'a Run, rule: &str) -> Vec<&'a Finding> {
    run.findings
        .iter()
        .filter(|f| f.rule_id.as_str() == rule)
        .collect()
}

#[test]
fn a_paragraph_beside_many_fences_is_analysed_once() {
    let (_guard, run) = run();
    assert_eq!(run.analyses, 1, "{FENCES} fences share one paragraph");
}

#[test]
fn a_restatement_of_a_shared_paragraph_is_reported_once() {
    let (_guard, run) = run();
    let found = of_rule(&run, "diagrams:diagram.prose-restates-labels");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].matched_text, PARAGRAPH);
}

#[test]
fn a_meta_reference_in_a_shared_paragraph_is_reported_once() {
    let (_guard, run) = run();
    let found = of_rule(&run, "diagrams:diagram.meta-reference");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].matched_text, META);
}

#[test]
fn findings_retain_exactly_one_copy_of_the_paragraph() {
    let (_guard, run) = run();
    let retained: usize = run.findings.iter().map(|f| f.matched_text.len()).sum();
    assert_eq!(retained, PARAGRAPH.len() + META.len());
}

#[test]
fn a_repeated_paragraph_finding_is_not_built_for_every_fence() {
    let (_guard, run) = run();
    assert_eq!(
        run.built_bytes,
        PARAGRAPH.len() + META.len(),
        "{FENCES} fences share one paragraph"
    );
}
