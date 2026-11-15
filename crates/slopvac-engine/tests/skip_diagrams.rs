//! Skip ranges and diagram rules.

#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs,
    clippy::single_range_in_vec_init
)]

mod common;

use common::{category, id, ruleset, settings};
use slopvac_diagrams::Format;
use slopvac_engine::{
    DIAGRAM_PACK, DiagramChecker, DiagramHit, DiagramInput, DiagramRule, Engine, Input, Level,
    diagram_rule_set, skip_ranges,
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
fn skip_ranges_hold_list_items_and_quotes() {
    let source = "- item\n\n> quote\n\nplain `c` text\n";
    let doc = parse_markdown(source, Span::new(0, source.len()).unwrap()).unwrap();
    let skip = skip_ranges(&doc, &[BlockKind::ListItem, BlockKind::Quote], None);
    // "item" is bytes 2 to 6, "quote" is 10 to 15 and the code span `c` is 23 to 26.
    assert_eq!(skip.to_ranges(), [2..6, 10..15, 23..26]);
}

const MERMAID: &str = "Intro text.\n\n```mermaid\nflowchart TD\n  A[This label has far too many words in it] --> B[End]\n```\n";

fn diagram_engine(settings: slopvac_engine::Settings) -> Engine {
    let pack = diagram_rule_set(&PackId::new(DIAGRAM_PACK).unwrap()).unwrap();
    Engine::new(vec![ruleset(vec![category("cat", &[])]), pack], settings).unwrap()
}

#[test]
fn a_fenced_diagram_is_checked_and_positioned_in_the_host_file() {
    let doc = diagram_engine(settings())
        .lint(&Input::markdown("a.md", MERMAID))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    let f = &doc.findings[0];
    assert_eq!(f.rule_id, id("diagrams:diagram.label-run-on"));
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
        .insert(id("diagrams:diagram.label-run-on"), Level::Off);
    assert!(
        diagram_engine(off)
            .lint(&Input::markdown("a.md", MERMAID))
            .unwrap()
            .findings
            .is_empty()
    );
    let mut error = settings();
    error.rule_levels.insert(
        id("diagrams:diagram.label-run-on"),
        Level::On(Severity::Error),
    );
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

#[test]
fn the_shipped_pack_holds_five_signal_rules_and_three_judgement_rules() {
    let set = diagram_rule_set(&PackId::new(DIAGRAM_PACK).unwrap()).unwrap();
    let mut got: Vec<_> = set
        .rules()
        .map(|r| (r.id().to_string(), r.rule().severity))
        .collect();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    let want = [
        ("diagrams:diagram.label-run-on", Severity::Warning),
        (
            "diagrams:diagram.label-title-plus-gloss",
            Severity::Suggestion,
        ),
        ("diagrams:diagram.meta-reference", Severity::Warning),
        ("diagrams:diagram.note-long", Severity::Warning),
        ("diagrams:diagram.note-needed", Severity::Suggestion),
        ("diagrams:diagram.over-explained", Severity::Suggestion),
        ("diagrams:diagram.prose-restates", Severity::Suggestion),
        ("diagrams:diagram.prose-restates-labels", Severity::Warning),
    ];
    assert_eq!(got, want.map(|(i, s)| (i.to_owned(), s)));
}

#[test]
fn the_pack_severity_is_the_shipped_level_and_a_category_level_replaces_it() {
    let source = "flowchart TD\n  A[This label has far too many words in it] --> B[End]\n";
    let input = Input::diagram("flow.mmd", source, Format::Mermaid);
    let shipped = diagram_engine(settings()).lint(&input).unwrap();
    assert_eq!(shipped.findings[0].severity, Severity::Warning);
    let mut capped = settings();
    capped.categories.insert(
        slopvac_model::CategoryId::new("diagram").unwrap(),
        slopvac_engine::CategoryDials {
            level: Some(Level::On(Severity::Suggestion)),
            ..Default::default()
        },
    );
    let doc = diagram_engine(capped).lint(&input).unwrap();
    assert_eq!(doc.findings[0].severity, Severity::Suggestion);
}

#[test]
fn a_title_plus_gloss_label_ships_as_a_suggestion() {
    let source = "flowchart TD\n  A[Clean Data: remove duplicates, nulls] --> B[End]\n";
    let doc = diagram_engine(settings())
        .lint(&Input::diagram("flow.mmd", source, Format::Mermaid))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(
        doc.findings[0].rule_id,
        id("diagrams:diagram.label-title-plus-gloss")
    );
    assert_eq!(doc.findings[0].severity, Severity::Suggestion);
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

const DIAGRAM: &str = "```mermaid\nflowchart TD\n  A[Start] --> B[End]\n```\n";

fn reference_texts(source: &str) -> Vec<(String, usize, usize)> {
    let doc = diagram_engine(settings())
        .lint(&Input::markdown("a.md", source))
        .unwrap();
    doc.findings
        .iter()
        .filter(|f| f.rule_id == id("diagrams:diagram.meta-reference"))
        .map(|f| {
            let span = f.extent.unwrap().span;
            (f.matched_text.clone(), span.start(), span.end())
        })
        .collect()
}

#[test]
fn a_meta_reference_after_accented_bold_text_is_placed_in_the_source() {
    // "**É** " is 7 bytes, so the phrase covers bytes 7 to 24.
    let source = format!("**É** the diagram below shows it.\n\n{DIAGRAM}");
    assert_eq!(
        reference_texts(&source),
        [("the diagram below".to_owned(), 7, 24)]
    );
}

#[test]
fn a_meta_reference_after_plain_markup_is_placed_in_the_source() {
    let source = format!("See **bold** and the diagram below.\n\n{DIAGRAM}");
    assert_eq!(
        reference_texts(&source),
        [("the diagram below".to_owned(), 17, 34)]
    );
}

#[test]
fn a_meta_reference_in_wrapped_prose_is_placed_in_the_source() {
    // The soft wrap is one byte in the source and one space in the prose.
    let source = format!("First line\nthe diagram below shows it.\n\n{DIAGRAM}");
    assert_eq!(
        reference_texts(&source),
        [("the diagram below".to_owned(), 11, 28)]
    );
}

#[test]
fn a_raw_rule_does_not_read_a_diagram_file() {
    let raw = common::rule(
        "raw",
        &serde_json::json!({"kind": "tokens", "tokens": ["Start"]}),
        &serde_json::json!({"scope": "raw"}),
    );
    let e = Engine::new(vec![ruleset(vec![category("cat", &[raw])])], settings()).unwrap();
    let source = "flowchart TD\n  A[Start] --> B[End]\n";
    let doc = e
        .lint(&Input::diagram("flow.mmd", source, Format::Mermaid))
        .unwrap();
    assert!(doc.findings.is_empty());
}

/// A checker that answers with a range built from the source length and the prose base.
type RangeFor = fn(usize) -> std::ops::Range<usize>;

struct Ranged(RangeFor);

impl DiagramChecker for Ranged {
    fn rules(&self) -> Vec<DiagramRule> {
        vec![DiagramRule {
            id: id("acme:diagram.ranged"),
            severity: Severity::Suggestion,
        }]
    }

    fn check(&self, input: &DiagramInput<'_>) -> Vec<DiagramHit> {
        vec![DiagramHit {
            rule: id("acme:diagram.ranged"),
            span: (self.0)(input.source.len()),
            message: "bad".into(),
        }]
    }
}

/// A range with `start > end`, which a range literal would not let clippy accept.
fn reversed(start: usize, end: usize) -> std::ops::Range<usize> {
    #[allow(clippy::reversed_empty_ranges)]
    {
        start..end
    }
}

#[test]
fn a_checker_range_that_is_reversed_or_out_of_bounds_is_an_error_with_no_findings() {
    // The prose before the diagram starts at `len + 1` in the checker's coordinates.
    let cases: [(&str, RangeFor); 5] = [
        ("reversed in the source", |_| reversed(5, 2)),
        ("past the end of the file", |len| len - 1..len + 1),
        ("start at the prose base, end below it", |len| {
            reversed(len + 1, len)
        }),
        ("reversed inside the prose", |len| {
            reversed(len + 6, len + 3)
        }),
        ("past the end of the prose", |len| len + 1..len + 1_000),
    ];
    for (name, range) in cases {
        let e = Engine::new(vec![ruleset(vec![category("cat", &[])])], settings())
            .unwrap()
            .with_diagram_checker(Box::new(Ranged(range)));
        let source = format!("Intro text here.\n\n{DIAGRAM}");
        let result = e.lint(&Input::markdown("a.md", &source));
        assert!(result.is_err(), "{name}: {result:?}");
    }
}
