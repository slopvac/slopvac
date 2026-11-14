//! Metric rules: a counted measurement compared with a threshold.
//!
//! Expected counts are worked out by hand from the metric definitions in the 2.x reference.

#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

mod common;

use common::{engine, rule};
use serde_json::{Value, json};
use slopvac_model::{Document, Severity};

/// A metric rule `cat.m` that says what it measured and the limit.
fn metric(name: &str, threshold: f64, comparison: &str, extra: &Value) -> Value {
    let mut extra = extra.clone();
    extra["message"] = json!("Got {match} over {replacement}.");
    rule(
        "m",
        &json!({"kind": "metric", "metric": name, "threshold": threshold, "comparison": comparison}),
        &extra,
    )
}

fn lint(rule: &Value, text: &str) -> Document {
    engine(&[rule.clone()])
        .lint(&slopvac_engine::Input::markdown("a.md", text))
        .unwrap()
}

/// `(line, message)` of each finding.
fn hits(doc: &Document) -> Vec<(usize, String)> {
    doc.findings
        .iter()
        .map(|f| (f.start.line(), f.message.as_str().to_owned()))
        .collect()
}

fn gt(name: &str, threshold: f64) -> Value {
    metric(name, threshold, "gt", &json!({"scope": "sentence"}))
}

#[test]
fn a_sentence_over_the_word_limit_is_reported_on_its_line_with_no_range() {
    let doc = lint(
        &gt("sentence_words", 5.0),
        "Short one.\nThis second sentence has six words.\n",
    );
    assert_eq!(hits(&doc), [(2, "Got 6 over 5.".to_owned())]);
    let f = &doc.findings[0];
    assert_eq!(
        (f.start.column(), f.extent, f.matched_text.as_str()),
        (1, None, "")
    );
    assert_eq!(f.severity, Severity::Warning);
    assert_eq!(doc.warnings, 1);
}

#[test]
fn a_word_limit_with_a_text_type_skips_sentences_of_another_type() {
    let rule = metric(
        "sentence_words",
        3.0,
        "gt",
        &json!({"scope": "sentence", "text_type": "procedural"}),
    );
    let doc = lint(
        &rule,
        "Open the valve and close the door.\n\nThe valve controls the flow of water.\n",
    );
    assert_eq!(hits(&doc), [(1, "Got 7 over 3.".to_owned())]);
}

#[test]
fn clause_boundaries_are_shown_as_clauses() {
    // Two semicolons are two boundaries, shown as 3 clauses against a limit of 1 + 1.
    let doc = lint(
        &gt("clause_boundaries", 1.0),
        "We read it; we parse it; we emit it.\n",
    );
    assert_eq!(hits(&doc), [(1, "Got 3 over 2.".to_owned())]);
}

#[test]
fn a_lead_in_counts_the_words_before_the_colon_and_needs_a_colon() {
    let text = "Check the oil level in the tank and then the water level in the radiator: both low.\n\n\
                Check the oil level in the tank and then the water level in the radiator today.\n";
    let doc = lint(&gt("lead_in_words", 10.0), text);
    assert_eq!(hits(&doc), [(1, "Got 15 over 10.".to_owned())]);
}

#[test]
fn a_noun_stack_is_measured_in_words() {
    let doc = lint(
        &gt("multiword_noun_words", 3.0),
        "The container orchestration platform migration strategy fails.\n",
    );
    assert_eq!(hits(&doc), [(1, "Got 5 over 3.".to_owned())]);
}

#[test]
fn a_series_is_measured_in_items() {
    let doc = lint(&gt("coordinated_items", 3.0), "We ship a, b, c, and d.\n");
    assert_eq!(hits(&doc), [(1, "Got 4 over 3.".to_owned())]);
}

#[test]
fn a_paragraph_counts_its_sentences_but_a_list_item_does_not() {
    let rule = metric(
        "paragraph_sentences",
        2.0,
        "gt",
        &json!({"scope": "paragraph"}),
    );
    let doc = lint(
        &rule,
        "One. Two. Three.\n\n- One. Two. Three.\n\n> One. Two. Three.\n",
    );
    assert_eq!(hits(&doc), [(1, "Got 3 over 2.".to_owned())]);
}

#[test]
fn a_long_paragraph_counts_only_when_three_sentences_hold_a_series() {
    let rule = metric(
        "enumerating_paragraph_words",
        10.0,
        "gt",
        &json!({"scope": "paragraph"}),
    );
    let three = "We buy a, b, and c. We sell d, e, and f. We keep g, h, and i.";
    let two = "We buy a, b, and c. We sell d, e, and f. We keep g and h today.";
    let doc = lint(&rule, &format!("{three}\n\n{two}\n"));
    // Each series sentence is six words; the paragraph is 18.
    assert_eq!(hits(&doc), [(1, "Got 18 over 10.".to_owned())]);
}

#[test]
fn a_document_metric_reports_on_the_first_line() {
    let doc = lint(
        &metric(
            "adjectives_per_noun",
            0.5,
            "gt",
            &json!({"scope": "document"}),
        ),
        "# Title\n\nA beautiful careless migration.\n",
    );
    // `beautiful` and `careless` are adjectives; `migration` is the one noun.
    assert_eq!(hits(&doc), [(1, "Got 2.00 over 0.50.".to_owned())]);
}

#[test]
fn dashes_are_counted_per_thousand_prose_words() {
    // Two dashes over four words: 500 per thousand.
    let doc = lint(
        &metric(
            "dash_per_1000_words",
            5.0,
            "gt",
            &json!({"scope": "document"}),
        ),
        "A — b. C — d.\n",
    );
    assert_eq!(hits(&doc), [(1, "Got 500.00 over 5.00.".to_owned())]);
}

#[test]
fn bold_spans_are_counted_per_thousand_prose_words_outside_code() {
    let rule = metric(
        "bold_spans_per_1000_words",
        5.0,
        "gt",
        &json!({"scope": "document"}),
    );
    // Two spans over four words.
    let doc = lint(&rule, "**one** **two** word word.\n");
    assert_eq!(hits(&doc), [(1, "Got 500.00 over 5.00.".to_owned())]);
    // A span in a code block is not markup.
    let doc = lint(&rule, "Word word word.\n\n```\n**a** **b**\n```\n");
    assert_eq!(hits(&doc), []);
}

#[test]
fn a_blank_line_does_not_break_a_run_of_bold_colon_bullets_but_text_does() {
    let rule = metric(
        "consecutive_bold_colon_bullets",
        3.0,
        "gte",
        &json!({"scope": "document"}),
    );
    let run = lint(&rule, "- **A:** x\n\n- **B:** y\n- __C__: z\n");
    assert_eq!(hits(&run), [(1, "Got 3.00 over 3.00.".to_owned())]);
    let broken = lint(&rule, "- **A:** x\n- **B:** y\n\nText.\n\n- **C:** z\n");
    assert_eq!(hits(&broken), []);
}

#[test]
fn the_comparison_decides_which_side_of_the_threshold_reports() {
    // Two words in the sentence.
    for (comparison, threshold, reported) in [
        ("gt", 2.0, false),
        ("gte", 2.0, true),
        ("lt", 2.0, false),
        ("lte", 2.0, true),
        ("lt", 3.0, true),
        ("gt", 1.0, true),
    ] {
        let rule = metric(
            "sentence_words",
            threshold,
            comparison,
            &json!({"scope": "sentence"}),
        );
        let doc = lint(&rule, "Two words.\n");
        assert_eq!(
            !doc.findings.is_empty(),
            reported,
            "{comparison} {threshold}"
        );
    }
}

#[test]
fn a_directive_suppresses_a_metric_finding_on_the_line_it_covers() {
    let text = "<!-- slopvac-disable-next-line -->\nThis second sentence has six words.\n\n\
                This third sentence has six words.\n";
    let doc = lint(&gt("sentence_words", 5.0), text);
    assert_eq!(hits(&doc), [(4, "Got 6 over 5.".to_owned())]);
}

#[test]
fn a_metric_the_engine_does_not_measure_is_reported_as_not_run() {
    let doc = lint(&gt("syllables_per_word", 2.0), "Anything at all.\n");
    assert!(doc.findings.is_empty());
    assert_eq!(doc.unchecked.len(), 1);
    assert!(
        doc.unchecked[0]
            .message
            .as_str()
            .starts_with("1 active metric rule(s) did not run")
    );
}
