//! Which paragraphs a diagram checker sees as nearby prose, and that finding them scales.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::engine;
use slopvac_engine::{DiagramChecker, DiagramHit, DiagramInput, DiagramRule, Input};

/// Records the text of the nearby paragraphs of each diagram it checks.
struct Recorder(Arc<Mutex<Vec<Vec<String>>>>);

impl DiagramChecker for Recorder {
    fn rules(&self) -> Vec<DiagramRule> {
        Vec::new()
    }

    fn check(&self, input: &DiagramInput<'_>) -> Vec<DiagramHit> {
        let seen = input.prose.iter().map(|p| p.text.to_owned()).collect();
        self.0.lock().unwrap().push(seen);
        Vec::new()
    }
}

/// The nearby paragraph texts of each fenced diagram in `source`, in document order.
fn nearby(source: &str) -> Vec<Vec<String>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let engine = engine(&[]).with_diagram_checker(Box::new(Recorder(Arc::clone(&seen))));
    engine.lint(&Input::markdown("a.md", source)).unwrap();
    seen.lock().unwrap().clone()
}

const FENCE: &str = "```mermaid\nflowchart TD\n```\n";

#[test]
fn a_fence_sees_the_paragraph_before_and_the_paragraph_after() {
    let source = format!("Before.\n\n{FENCE}\nAfter.\n");
    assert_eq!(nearby(&source), [["Before.", "After."]]);
}

#[test]
fn a_fence_with_no_paragraph_before_sees_only_the_one_after() {
    let source = format!("{FENCE}\nAfter.\n");
    assert_eq!(nearby(&source), [["After."]]);
}

#[test]
fn a_fence_with_no_paragraph_after_sees_only_the_one_before() {
    let source = format!("Before.\n\n{FENCE}");
    assert_eq!(nearby(&source), [["Before."]]);
}

#[test]
fn a_fence_with_no_paragraph_sees_nothing() {
    assert_eq!(nearby(FENCE), [Vec::<String>::new()]);
}

#[test]
fn each_fence_sees_the_paragraphs_that_border_it() {
    let source = format!("One.\n\n{FENCE}\nTwo.\n\n{FENCE}\nThree.\n");
    assert_eq!(nearby(&source), [["One.", "Two."], ["Two.", "Three."]]);
}

#[test]
fn fences_with_no_paragraph_between_them_share_their_neighbours() {
    let source = format!("One.\n\n{FENCE}\n{FENCE}\nTwo.\n");
    assert_eq!(nearby(&source), [["One.", "Two."], ["One.", "Two."]]);
}

#[test]
fn a_block_that_is_not_a_paragraph_is_not_nearby_prose() {
    let source = format!("One.\n\n# Heading\n\n{FENCE}\n# Other\n");
    assert_eq!(nearby(&source), [["One."]]);
}

#[test]
fn a_fence_sees_only_the_closest_projected_paragraphs() {
    let source = format!("Earlier.\n\n**Before**.\n\n{FENCE}\n*After*.\n\nLater.\n");
    assert_eq!(nearby(&source), [["Before.", "After."]]);
}

#[test]
fn nested_fences_share_neighbours_across_container_blocks() {
    let source = "Before.\n\n> ```mermaid\n> flowchart TD\n> ```\n>\n> ```mermaid\n> flowchart TD\n> ```\n\nAfter.\n";
    assert_eq!(
        nearby(source),
        [["Before.", "After."], ["Before.", "After."]]
    );
}

/// Time to lint a run of diagrams, optionally bordered by paragraphs.
fn lint_time(fences: usize, with_paragraphs: bool) -> Duration {
    let mut source = FENCE.repeat(fences);
    if with_paragraphs {
        source = format!("Before.\n\n{source}\nAfter.\n");
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let engine = engine(&[]).with_diagram_checker(Box::new(Recorder(Arc::clone(&seen))));
    let start = Instant::now();
    engine.lint(&Input::markdown("a.md", &source)).unwrap();
    let took = start.elapsed();
    assert_eq!(seen.lock().unwrap().len(), fences);
    took
}

#[test]
fn finding_nearby_prose_grows_about_linearly_with_the_fence_count() {
    // Best of five runs reduces scheduling noise. Setup is outside the timed region.
    for with_paragraphs in [false, true] {
        let best = |n| (0..5).map(|_| lint_time(n, with_paragraphs)).min().unwrap();
        let small = best(1_000);
        let medium = best(2_000);
        let large = best(4_000);
        assert!(
            medium.as_secs_f64() < 3.5 * small.as_secs_f64()
                && large.as_secs_f64() < 3.5 * medium.as_secs_f64()
                && large.as_secs_f64() < 6.0 * small.as_secs_f64(),
            "with paragraphs {with_paragraphs}: 1k {small:?}, 2k {medium:?}, 4k {large:?}"
        );
    }
}
