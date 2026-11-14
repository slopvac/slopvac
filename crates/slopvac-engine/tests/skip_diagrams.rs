//! Skip ranges and diagram rules.

#![allow(missing_docs, clippy::single_range_in_vec_init)]

mod common;

use common::{category, id, ruleset, settings};
use slopvac_diagrams::Format;
use slopvac_engine::{
    DiagramChecker, DiagramHit, DiagramInput, DiagramRule, Engine, Input, Level, SignalChecker,
    skip_ranges,
};
use slopvac_model::{PackId, Severity, Span};
use slopvac_parse::parse_markdown;
use slopvac_rules::BlockKind;

const DOC: &str = "# Head\n\nSome `code` here.\n\n```\nfence\n```\n";

fn parsed() -> slopvac_parse::Document {
    parse_markdown(DOC, Span::new(0, DOC.len()).unwrap()).unwrap()
}

#[test]
fn skip_ranges_hold_excluded_blocks_and_inline_code() {
    let skip = skip_ranges(&parsed(), &[BlockKind::Heading, BlockKind::Code], None);
    assert_eq!(skip.to_ranges(), [0..6, 13..19, 27..40]);
}

#[test]
fn skip_ranges_always_hold_inline_code_and_add_front_matter_when_asked() {
    let front = Span::new(0, 4).unwrap();
    let skip = skip_ranges(&parsed(), &[BlockKind::FrontMatter], Some(front));
    assert_eq!(skip.to_ranges(), [0..4, 13..19]);
}

#[test]
fn skip_ranges_merge_overlaps_and_answer_overlap_queries() {
    let skip = skip_ranges(&parsed(), &[BlockKind::Heading], None);
    let at = |a, b| skip.overlaps(Span::new(a, b).unwrap());
    assert!(at(2, 3));
    assert!(at(5, 9), "partly inside the heading");
    assert!(!at(6, 8), "the range ends at 6");
    assert!(at(13, 14));
    assert!(!at(19, 25));
    assert!(at(15, 15), "an empty span inside a range counts");
}

#[test]
fn skip_ranges_name_the_kinds_the_parser_cannot_locate() {
    let skip = skip_ranges(&parsed(), &[BlockKind::ListItem, BlockKind::Quote], None);
    assert_eq!(skip.unsupported(), [BlockKind::ListItem, BlockKind::Quote]);
    assert_eq!(skip.to_ranges(), [13..19]);
}

const MERMAID: &str = "Intro text.\n\n```mermaid\nflowchart TD\n  A[This label has far too many words in it] --> B[End]\n```\n";

fn diagram_engine(settings: slopvac_engine::Settings) -> Engine {
    Engine::new(vec![ruleset(vec![category("cat", &[])])], settings)
        .unwrap()
        .with_diagram_checker(Box::new(SignalChecker::new(PackId::new("core").unwrap())))
}

#[test]
fn a_fenced_diagram_is_checked_and_positioned_in_the_host_file() {
    let doc = diagram_engine(settings())
        .lint(&Input::markdown("a.md", MERMAID))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    let f = &doc.findings[0];
    assert_eq!(f.rule_id, id("core:diagram.label-run-on"));
    assert_eq!(f.category.as_str(), "diagram");
    assert_eq!(f.severity, Severity::Warning);
    assert_eq!(f.matched_text, "This label has far too many words in it");
    // Line 5 is `  A[This ...`, and the label starts after the two spaces and `A[`.
    assert_eq!((f.start.line(), f.start.column()), (5, 5));
    assert!(f.contexts.iter().any(|c| c.as_str() == "diagram-label"));
}

#[test]
fn a_diagram_file_is_checked_without_prose() {
    let source = "flowchart TD\n  A[This label has far too many words in it] --> B[End]\n";
    let doc = diagram_engine(settings())
        .lint(&Input::diagram("flow.mmd", source, Format::Mermaid))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(
        (doc.findings[0].start.line(), doc.findings[0].start.column()),
        (2, 5)
    );
}

#[test]
fn a_diagram_rule_obeys_its_level_and_a_disabled_region() {
    let mut off = settings();
    off.rule_levels
        .insert(id("core:diagram.label-run-on"), Level::Off);
    assert!(
        diagram_engine(off)
            .lint(&Input::markdown("a.md", MERMAID))
            .unwrap()
            .findings
            .is_empty()
    );
    let mut error = settings();
    error
        .rule_levels
        .insert(id("core:diagram.label-run-on"), Level::On(Severity::Error));
    let doc = diagram_engine(error)
        .lint(&Input::markdown("a.md", MERMAID))
        .unwrap();
    assert_eq!(doc.findings[0].severity, Severity::Error);
    let hidden = format!("<!-- slopvac-disable -->\n\n{MERMAID}");
    assert!(
        diagram_engine(settings())
            .lint(&Input::markdown("a.md", &hidden))
            .unwrap()
            .findings
            .is_empty()
    );
}

#[test]
fn a_fence_without_a_diagram_language_is_not_a_diagram() {
    let plain = MERMAID.replace("mermaid", "text");
    assert!(
        diagram_engine(settings())
            .lint(&Input::markdown("a.md", &plain))
            .unwrap()
            .findings
            .is_empty()
    );
}

/// A checker that flags every fragment, to show the dispatch hands over each one.
struct EveryFragment;

impl DiagramChecker for EveryFragment {
    fn rules(&self) -> Vec<DiagramRule> {
        vec![DiagramRule {
            id: id("acme:diagram.every"),
            severity: Severity::Suggestion,
        }]
    }

    fn check(&self, input: &DiagramInput<'_>) -> Vec<DiagramHit> {
        input
            .extraction
            .fragments
            .iter()
            .map(|f| DiagramHit {
                rule: id("acme:diagram.every"),
                span: f.span.clone(),
                message: "label".into(),
            })
            .collect()
    }
}

#[test]
fn a_custom_checker_reports_under_its_own_pack() {
    let e = Engine::new(vec![ruleset(vec![category("cat", &[])])], settings())
        .unwrap()
        .with_diagram_checker(Box::new(EveryFragment));
    let doc = e.lint(&Input::markdown("a.md", MERMAID)).unwrap();
    let texts: Vec<_> = doc
        .findings
        .iter()
        .map(|f| f.matched_text.as_str())
        .collect();
    assert_eq!(texts, ["This label has far too many words in it", "End"]);
    assert!(doc.findings.iter().all(|f| f.pack.as_str() == "acme"));
}
