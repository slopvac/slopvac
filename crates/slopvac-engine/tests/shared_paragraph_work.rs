//! One long paragraph beside many adjacent fences is analysed once and reported once.
//!
//! The tests count paragraph analyses (`test-counters`) and retained bytes rather than timing
//! the lint, so a loaded runner cannot make them fail. The counter is process-wide, so the
//! tests take a lock and read a difference.

#![allow(missing_docs)]

mod common;

use std::sync::{Mutex, MutexGuard};

use common::{category, ruleset, settings};
use slopvac_diagrams::prose_analyses;
use slopvac_engine::{DIAGRAM_PACK, Engine, Input, diagram_rule_set};
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

struct Run {
    analyses: usize,
    findings: Vec<Finding>,
}

fn run() -> (MutexGuard<'static, ()>, Run) {
    let guard = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let source = document();
    let before = prose_analyses();
    let findings = engine()
        .lint(&Input::markdown("a.md", &source))
        .unwrap()
        .findings;
    let analyses = prose_analyses() - before;
    (guard, Run { analyses, findings })
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
    assert_eq!(found[0].matched_text, "the diagram below");
}

#[test]
fn findings_retain_no_more_than_one_copy_of_the_paragraph() {
    let (_guard, run) = run();
    let retained: usize = run.findings.iter().map(|f| f.matched_text.len()).sum();
    // One copy of the paragraph, and the short matched text of the meta reference.
    assert!(
        retained <= PARAGRAPH.len() + 64,
        "{retained} bytes retained for a {} byte paragraph",
        PARAGRAPH.len()
    );
}
