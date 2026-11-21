//! Skip ranges and diagram rules.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(
    clippy::allow_attributes_without_reason,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]
#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs,
    clippy::single_range_in_vec_init
)]

mod common;

use common::{category, id, ruleset, settings};
use rstest::rstest;
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

fn sees_diagram_label(doc: &slopvac_model::Document) -> bool {
    doc.contexts_seen
        .iter()
        .any(|c| c.as_str() == "diagram-label")
}

const CLEAN_FENCE: &str = "Intro.\n\n```mermaid\nflowchart TD\n  A[Start] --> B[End]\n```\n";
const CLEAN_FILE: &str = "flowchart TD\n  A[Start] --> B[End]\n";
const DIAGRAM_RULES: [&str; 8] = [
    "label-run-on",
    "label-title-plus-gloss",
    "meta-reference",
    "note-long",
    "note-needed",
    "over-explained",
    "prose-restates",
    "prose-restates-labels",
];

fn no_checker() -> Engine {
    Engine::new(vec![ruleset(vec![category("cat", &[])])], settings()).unwrap()
}

fn rules_disabled() -> Engine {
    let mut off = settings();
    for rule in DIAGRAM_RULES {
        off.rule_levels
            .insert(id(&format!("diagrams:diagram.{rule}")), Level::Off);
    }
    diagram_engine(off)
}

/// The default genre is `reference`, so a rule limited to `consumer` does not apply.
fn genre_excludes_checker() -> Engine {
    let rule = signal_rule(
        "label-run-on",
        "diagram-label-run-on",
        &serde_json::json!({"applies_to": {"genres": ["consumer"]}}),
    );
    one_rule_engine(vec![("diagram", &rule)])
}

#[rstest]
#[case::no_checker(no_checker())]
#[case::rules_disabled(rules_disabled())]
#[case::genre_excludes_checker(genre_excludes_checker())]
#[case::checker_runs(diagram_engine(settings()))]
fn contexts_seen_lists_the_diagram_context_whether_or_not_a_checker_runs(#[case] engine: Engine) {
    let fenced = engine.lint(&Input::markdown("a.md", CLEAN_FENCE)).unwrap();
    assert!(sees_diagram_label(&fenced), "fenced diagram");
    let file = engine
        .lint(&Input::diagram("flow.mmd", CLEAN_FILE, Format::Mermaid))
        .unwrap();
    assert!(sees_diagram_label(&file), "diagram file");
}

#[test]
fn contexts_seen_lists_every_context_of_a_document_in_order() {
    let pack = diagram_rule_set(&PackId::new(DIAGRAM_PACK).unwrap()).unwrap();
    let dull = common::tokens("dull", "leverage", &serde_json::json!({}));
    let engine = Engine::new(
        vec![ruleset(vec![category("cat", &[dull])]), pack],
        settings(),
    )
    .unwrap();
    let source =
        "You must leverage the cache.\n\n```mermaid\nflowchart TD\n  A[Start] --> B[End]\n```\n";
    let doc = engine.lint(&Input::markdown("a.md", source)).unwrap();
    let seen: Vec<&str> = doc
        .contexts_seen
        .iter()
        .map(slopvac_model::ContextTag::as_str)
        .collect();
    // Sorted by name: the prose context comes after the diagram one.
    assert_eq!(seen, ["diagram-label", "requirement"]);
}

#[test]
fn contexts_seen_omits_the_diagram_context_without_a_diagram() {
    let doc = diagram_engine(settings())
        .lint(&Input::markdown("a.md", "Just prose here.\n"))
        .unwrap();
    assert!(!sees_diagram_label(&doc));
}

#[test]
fn a_quoted_fence_is_positioned_past_the_quote_markers() {
    let source = "> ```mermaid\n> sequenceDiagram\n> participant A as This label has far too many words in it\n> ```\n";
    let doc = diagram_engine(settings())
        .lint(&Input::markdown("a.md", source))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    let f = &doc.findings[0];
    assert_eq!(f.matched_text, "This label has far too many words in it");
    // Line 3 is `> participant A as This ...`: the label starts after 19 bytes, so at column 20.
    assert_eq!((f.start.line(), f.start.column()), (3, 20));
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
        slopvac_engine::CategoryDials::default().with_level(Some(Level::On(Severity::Suggestion))),
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

/// A draw.io file whose one page is compressed. Its label is
/// `This label has far too many words in it`.
const COMPRESSED_PAGE: &str = "jVBLDgIxCD0N+057hPpZufMCmGKmSVsmWLXeXmpNNCaTuODx4AEBwPnc9oLLfOBACdwWnBfmOlhunlICa2IAtwFrjRrY3Yo6vVSzoFCp/zTgaLhhutLIHOd40UzCE/XCGXt0RlGszIoZy0PdnSV0KZYO9T2HpFJb3WX63kXJ504Nft7wBA==";

#[test]
fn a_compressed_drawio_label_reports_its_text_at_the_payload_span() {
    let source = format!("<mxfile><diagram name=\"P\">{COMPRESSED_PAGE}</diagram></mxfile>");
    let doc = diagram_engine(settings())
        .lint(&Input::diagram("a.drawio", &source, Format::Drawio))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    let f = &doc.findings[0];
    assert_eq!(f.rule_id, id("diagrams:diagram.label-run-on"));
    assert_eq!(f.matched_text, "This label has far too many words in it");
    let start = source.find(COMPRESSED_PAGE).unwrap();
    let span = f.extent.unwrap().span;
    assert_eq!(
        (span.start(), span.end()),
        (start, start + COMPRESSED_PAGE.len())
    );
}

#[test]
fn a_compressed_drawio_message_fills_match_with_the_decoded_label() {
    let rule = common::rule(
        "label-run-on",
        &serde_json::json!({"kind": "structure", "check": "diagram-label-run-on"}),
        &serde_json::json!({"message": "Cut {match}.", "scope": "prose"}),
    );
    let engine = Engine::new(
        vec![ruleset(vec![category("diagram", &[rule])])],
        settings(),
    )
    .unwrap();
    let source = format!("<mxfile><diagram name=\"P\">{COMPRESSED_PAGE}</diagram></mxfile>");
    let doc = engine
        .lint(&Input::diagram("a.drawio", &source, Format::Drawio))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(
        doc.findings[0].message.as_str(),
        "Cut This label has far too many words in it."
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
                matched: None,
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
            matched: None,
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
fn signal_rule(id: &str, check: &str, extra: &serde_json::Value) -> serde_json::Value {
    common::rule(
        id,
        &serde_json::json!({"kind": "structure", "check": check}),
        extra,
    )
}

fn found(engine: &Engine, input: &Input<'_>) -> Vec<String> {
    engine
        .lint(input)
        .unwrap()
        .findings
        .iter()
        .map(|f| f.rule_id.to_string())
        .collect()
}

#[test]
fn an_allow_with_a_listed_exception_suppresses_a_dispatched_diagram_rule() {
    let rule = signal_rule(
        "meta-reference",
        "diagram-meta-reference",
        &serde_json::json!({"exceptions": [{"name": "figure-caption"}]}),
    );
    let engine = Engine::new(
        vec![ruleset(vec![category("diagram", &[rule])])],
        settings(),
    )
    .unwrap();
    let allow = "<!-- slopvac-allow: rule=core:diagram.meta-reference reason=figure-caption -->\n";
    let text = format!("See the diagram below for details.\n\n{DIAGRAM}");
    assert_eq!(
        found(&engine, &Input::markdown("a.md", &text)),
        ["core:diagram.meta-reference"]
    );
    let allowed = format!("{allow}{text}");
    assert_eq!(
        found(&engine, &Input::markdown("a.md", &allowed)),
        Vec::<String>::new()
    );
}

#[test]
fn note_long_flags_a_six_word_note_and_not_a_five_word_one() {
    let note = |n: &str| format!("sequenceDiagram\n  participant A\n  Note right of A: {n}\n");
    let engine = diagram_engine(settings());
    let flag = |n: &str| found(&engine, &Input::diagram("s.mmd", &note(n), Format::Mermaid));
    assert_eq!(
        flag("keys rotate every night at midnight"),
        ["diagrams:diagram.note-long"]
    );
    assert_eq!(flag("keys rotate every night here"), Vec::<String>::new());
}

#[test]
fn prose_that_restates_the_labels_is_flagged_and_prose_that_adds_a_reason_is_not() {
    let labels = "flowchart TD\n  A[Load Balancer] --> B[API Gateway]\n  B --> C[Auth Service]\n  C --> D[User Database]\n";
    let restating = "Requests reach the load balancer first. The balancer forwards each request to the API gateway. The gateway asks the auth service to check the token, and then the user database returns the account record for that user. Responses travel back through the same path to the client over the network connection.";
    let adding = "We cap each tenant at 200 requests per second because the shared cache saturates beyond that point. When the cache fails, reads fall back to the primary at a latency cost of roughly 40 milliseconds, so alerts fire on the hit ratio instead of errors.";
    let engine = diagram_engine(settings());
    let doc = |prose: &str| format!("{prose}\n\n```mermaid\n{labels}```\n");
    assert_eq!(
        found(&engine, &Input::markdown("a.md", &doc(restating))),
        ["diagrams:diagram.prose-restates-labels"]
    );
    assert_eq!(
        found(&engine, &Input::markdown("a.md", &doc(adding))),
        Vec::<String>::new()
    );
}

#[test]
fn an_inactive_rule_does_not_hide_a_later_rule_with_the_same_check() {
    let first = signal_rule(
        "label-run-on",
        "diagram-label-run-on",
        &serde_json::json!({}),
    );
    let second = signal_rule("long-label", "diagram-label-run-on", &serde_json::json!({}));
    let mut s = settings();
    s.rule_levels
        .insert(id("core:diagram.label-run-on"), Level::Off);
    let engine = Engine::new(
        vec![ruleset(vec![
            category("diagram", &[first]),
            category("extra", &[second]),
        ])],
        s,
    )
    .unwrap();
    assert_eq!(
        found(&engine, &Input::markdown("a.md", MERMAID)),
        ["core:extra.long-label"]
    );
}
fn one_rule_engine(rules: Vec<(&str, &serde_json::Value)>) -> Engine {
    let sets = rules
        .into_iter()
        .map(|(category_id, rule)| ruleset(vec![category(category_id, &[rule.clone()])]))
        .collect();
    Engine::new(sets, settings()).unwrap()
}

#[test]
fn a_replaced_rule_id_reports_once_under_the_last_definition() {
    let run_on = signal_rule(
        "label-run-on",
        "diagram-label-run-on",
        &serde_json::json!({}),
    );
    let engine = one_rule_engine(vec![("diagram", &run_on), ("diagram", &run_on)]);
    assert_eq!(
        found(&engine, &Input::markdown("a.md", MERMAID)),
        ["core:diagram.label-run-on"]
    );
}

#[test]
fn a_replaced_rule_id_does_not_keep_reporting_its_old_signal() {
    let run_on = signal_rule(
        "label-run-on",
        "diagram-label-run-on",
        &serde_json::json!({}),
    );
    let note = signal_rule("label-run-on", "diagram-note-long", &serde_json::json!({}));
    let engine = one_rule_engine(vec![("diagram", &run_on), ("diagram", &note)]);
    assert_eq!(
        found(&engine, &Input::markdown("a.md", MERMAID)),
        Vec::<String>::new()
    );
}

#[test]
fn a_detector_exception_drops_a_quoted_diagram_reference() {
    let rule = signal_rule(
        "meta-reference",
        "diagram-meta-reference",
        &serde_json::json!({"exceptions": [{"name": "quotation", "detector": "quotation"}]}),
    );
    let engine = one_rule_engine(vec![("diagram", &rule)]);
    let lint = |text: &str| {
        found(
            &engine,
            &Input::markdown("a.md", &format!("{text}\n\n{DIAGRAM}")),
        )
    };
    assert_eq!(
        lint("Never write \"the diagram below\" in a doc."),
        Vec::<String>::new()
    );
    assert_eq!(
        lint("See the diagram below for details."),
        ["core:diagram.meta-reference"]
    );
}

#[test]
fn a_context_gate_drops_a_diagram_reference_in_a_quote_block() {
    let rule = signal_rule(
        "meta-reference",
        "diagram-meta-reference",
        &serde_json::json!({"contexts": {"skip": ["quote"]}}),
    );
    let engine = one_rule_engine(vec![("diagram", &rule)]);
    let lint = |text: &str| {
        found(
            &engine,
            &Input::markdown("a.md", &format!("{text}\n\n{DIAGRAM}")),
        )
    };
    assert_eq!(
        lint("> See the diagram below for details."),
        Vec::<String>::new()
    );
    assert_eq!(
        lint("See the diagram below for details."),
        ["core:diagram.meta-reference"]
    );
}

#[test]
fn a_finding_carries_the_message_of_the_loaded_rule() {
    let rule = signal_rule(
        "label-run-on",
        "diagram-label-run-on",
        &serde_json::json!({"message": "Shorten {match}."}),
    );
    let engine = one_rule_engine(vec![("diagram", &rule)]);
    let doc = engine.lint(&Input::markdown("a.md", MERMAID)).unwrap();
    assert_eq!(
        doc.findings[0].message.as_str(),
        "Shorten This label has far too many words in it."
    );
}
const LONG_LABEL_FILE: &str =
    "flowchart TD\n  A[This label has far too many words in it] --> B[End]\n";

fn run_on_with(extra: &serde_json::Value) -> Engine {
    let rule = signal_rule("label-run-on", "diagram-label-run-on", extra);
    one_rule_engine(vec![("diagram", &rule)])
}

fn both_inputs(engine: &Engine) -> [Vec<String>; 2] {
    [
        found(
            engine,
            &Input::diagram("flow.mmd", LONG_LABEL_FILE, Format::Mermaid),
        ),
        found(engine, &Input::markdown("a.md", MERMAID)),
    ]
}

#[test]
fn a_gate_that_needs_the_diagram_label_context_keeps_the_hit() {
    let engine = run_on_with(&serde_json::json!({"contexts": {"only": ["diagram-label"]}}));
    for got in both_inputs(&engine) {
        assert_eq!(got, ["core:diagram.label-run-on"]);
    }
}

#[test]
fn a_gate_that_skips_the_diagram_label_context_drops_the_hit() {
    let engine = run_on_with(&serde_json::json!({"contexts": {"skip": ["diagram-label"]}}));
    for got in both_inputs(&engine) {
        assert_eq!(got, Vec::<String>::new());
    }
}

#[test]
fn a_detector_named_for_the_diagram_label_context_drops_the_hit() {
    let engine = run_on_with(&serde_json::json!({
        "exceptions": [{"name": "labels", "detector": "diagram-label"}]
    }));
    for got in both_inputs(&engine) {
        assert_eq!(got, Vec::<String>::new());
    }
}

#[test]
fn the_ai_signal_of_a_diagram_rule_reaches_the_finding_and_the_summary() {
    let engine = run_on_with(&serde_json::json!({
        "ai_signal": "strong", "ai_signal_source": "measured"
    }));
    let doc = engine.lint(&Input::markdown("a.md", MERMAID)).unwrap();
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(doc.findings[0].ai_signal, slopvac_model::AiSignal::Strong);
    assert_eq!(doc.ai_register.strong.findings, 1);
    assert_eq!(doc.ai_register.strong.warnings, 1);
    assert_eq!(doc.prose.findings, 0);
}

/// A component whose label holds a literal backslash and `n`, written `\n`. Only `PlantUML` renders
/// that escape as a line break, so the label is a two-word name plus a four-word gloss.
const ESCAPED_BREAK_LABEL: &str = r"Order Service\nhandles checkout and refunds";

fn one_gloss_finding(doc: &slopvac_model::Document, line: usize, column: usize) {
    assert_eq!(doc.findings.len(), 1, "{:?}", doc.findings);
    let f = &doc.findings[0];
    assert_eq!(f.rule_id, id("diagrams:diagram.label-title-plus-gloss"));
    assert_eq!(f.matched_text, ESCAPED_BREAK_LABEL);
    assert_eq!((f.start.line(), f.start.column()), (line, column));
}

#[test]
fn an_escaped_break_in_a_puml_file_is_a_title_plus_gloss_label() {
    let source = format!("@startuml\ncomponent [{ESCAPED_BREAK_LABEL}] as W\n@enduml\n");
    let doc = diagram_engine(settings())
        .lint(&Input::diagram("c.puml", &source, Format::PlantUml))
        .unwrap();
    // Line 2 is `component [Order ...`: the label starts after 11 bytes, so at column 12.
    one_gloss_finding(&doc, 2, 12);
}

#[test]
fn an_escaped_break_in_a_fenced_plantuml_block_is_a_title_plus_gloss_label() {
    let source = format!(
        "Intro text.\n\n```plantuml\n@startuml\ncomponent [{ESCAPED_BREAK_LABEL}] as W\n@enduml\n```\n"
    );
    let doc = diagram_engine(settings())
        .lint(&Input::markdown("a.md", &source))
        .unwrap();
    // Line 5 is the component line, so the label sits at column 12 as in the standalone file.
    one_gloss_finding(&doc, 5, 12);
}

#[test]
fn an_escaped_break_in_a_mermaid_label_is_not_a_break() {
    let source = format!("flowchart TD\n  A[{ESCAPED_BREAK_LABEL}] --> B[End]\n");
    let doc = diagram_engine(settings())
        .lint(&Input::diagram("flow.mmd", &source, Format::Mermaid))
        .unwrap();
    assert_eq!(doc.findings.len(), 0, "{:?}", doc.findings);
}
