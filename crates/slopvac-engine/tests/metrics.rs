//! Metric rules: a counted measurement compared with a threshold.
//!
//! Expected counts are worked out by hand from the metric definitions in the 2.x reference.
#![cfg(test)]

#[expect(
    dead_code,
    reason = "each test binary uses the part of the shared helpers it needs"
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
    engine(std::slice::from_ref(rule))
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
fn a_raw_html_block_is_one_paragraph_for_the_paragraph_metrics() {
    let rule = metric(
        "paragraph_sentences",
        2.0,
        "gt",
        &json!({"scope": "paragraph"}),
    );
    // Two cells of two sentences each: four sentences in the one block, as in 2.x.
    let doc = lint(
        &rule,
        "<table>\n<tr><td>One. Two.</td><td>Three. Four.</td></tr>\n</table>\n",
    );
    assert_eq!(hits(&doc), [(2, "Got 4 over 2.".to_owned())]);
}

#[test]
fn a_sentence_metric_reads_a_raw_html_block_as_one_text() {
    let rule = gt("sentence_words", 3.0);
    // "No period Next one." is one sentence of four words in 2.x, not two cells of two.
    let doc = lint(
        &rule,
        "<table>\n<tr><td>No period</td><td>Next one.</td></tr>\n</table>\n",
    );
    assert_eq!(hits(&doc), [(2, "Got 4 over 3.".to_owned())]);
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
    assert_eq!(doc.findings, []);
    assert_eq!(doc.unchecked.len(), 1);
    assert!(
        doc.unchecked[0]
            .message
            .as_str()
            .starts_with("1 active metric rule(s) did not run")
    );
}

#[test]
fn a_zero_threshold_falls_back_to_the_word_cap_of_the_sentence_text_type() {
    let rule = gt("sentence_words", 0.0);
    // Procedural: 24 words against the cap of 20.
    let procedural = "Open the valve and then close the door and then check the gauge and then record \
                      the reading in the log for the shift.\n";
    assert_eq!(
        hits(&lint(&rule, procedural)),
        [(1, "Got 24 over 20.".to_owned())]
    );
    // Descriptive: 24 words are under its cap of 25, and 26 are over.
    let under = "The valve controls the flow of water through the main pipe and the gauge shows the \
                 pressure of the water in the pipe.\n";
    assert_eq!(hits(&lint(&rule, under)), []);
    let over = "The valve controls the flow of water through the main pipe and the gauge shows the \
                pressure of the water in the pipe at all times.\n";
    assert_eq!(
        hits(&lint(&rule, over)),
        [(1, "Got 26 over 25.".to_owned())]
    );
}

#[test]
fn a_rule_that_excludes_headings_does_not_report_a_long_heading() {
    let rule = metric(
        "sentence_words",
        3.0,
        "gt",
        &json!({"scope": "sentence", "exclude_blocks": ["heading"]}),
    );
    let doc = lint(
        &rule,
        "# This heading has six words.\n\nThis body has five.\n",
    );
    assert_eq!(hits(&doc), [(3, "Got 4 over 3.".to_owned())]);
}

#[test]
fn a_context_gate_selects_the_sentences_a_metric_reads() {
    let text = "You must open the valve now.\n\nWe open the valve now.\n";
    let skip = metric(
        "sentence_words",
        3.0,
        "gt",
        &json!({"scope": "sentence", "contexts": {"skip": ["requirement"]}}),
    );
    assert_eq!(hits(&lint(&skip, text)), [(3, "Got 5 over 3.".to_owned())]);
    let only = metric(
        "sentence_words",
        3.0,
        "gt",
        &json!({"scope": "sentence", "contexts": {"only": ["requirement"]}}),
    );
    assert_eq!(hits(&lint(&only, text)), [(1, "Got 6 over 3.".to_owned())]);
}

#[test]
fn an_exception_detector_drops_the_metric_finding_it_covers() {
    let rule = metric(
        "sentence_words",
        3.0,
        "gt",
        &json!({"scope": "sentence",
                "exceptions": [{"name": "requirement", "detector": "requirement"}]}),
    );
    let doc = lint(
        &rule,
        "You must open the valve now.\n\nWe open the valve now.\n",
    );
    assert_eq!(hits(&doc), [(3, "Got 5 over 3.".to_owned())]);
}
fn paragraph_rule(extra: &Value) -> Value {
    let mut extra = extra.clone();
    extra["scope"] = json!("paragraph");
    metric("paragraph_sentences", 2.0, "gt", &extra)
}

// The first paragraph sits in a section tagged `change-note`, the second does not.
const SECTIONS: &str =
    "# Doc\n\n## Breaking changes\n\nOne. Two. Three.\n\n## Other\n\nOne. Two. Three.\n";

#[test]
fn a_context_gate_selects_the_paragraphs_a_metric_reads() {
    let skip = paragraph_rule(&json!({"contexts": {"skip": ["change-note"]}}));
    assert_eq!(
        hits(&lint(&skip, SECTIONS)),
        [(9, "Got 3 over 2.".to_owned())]
    );
    let only = paragraph_rule(&json!({"contexts": {"only": ["change-note"]}}));
    assert_eq!(
        hits(&lint(&only, SECTIONS)),
        [(5, "Got 3 over 2.".to_owned())]
    );
}

#[test]
fn an_exception_detector_drops_the_paragraph_finding_it_covers() {
    let rule = paragraph_rule(
        &json!({"exceptions": [{"name": "change-note", "detector": "change-note"}]}),
    );
    assert_eq!(
        hits(&lint(&rule, SECTIONS)),
        [(9, "Got 3 over 2.".to_owned())]
    );
}

#[test]
fn an_unguarded_paragraph_rule_reports_both_paragraphs() {
    let doc = lint(&paragraph_rule(&json!({})), SECTIONS);
    assert_eq!(hits(&doc).len(), 2);
}

#[test]
fn the_quotation_detector_drops_a_metric_finding_for_a_wholly_quoted_sentence() {
    let rule = metric(
        "multiword_noun_words",
        3.0,
        "gt",
        &json!({"scope": "sentence",
                "exceptions": [{"name": "quotation", "detector": "quotation"}]}),
    );
    let stack = "The container orchestration platform migration strategy.";
    let text = format!("Never write \"{stack}\" here.\n\n\"{stack}\"\n\n{stack}\n");
    let doc = lint(&rule, &text);
    // Line 1 is one sentence with a quotation inside it; line 3 is wholly quoted.
    assert_eq!(hits(&doc).iter().map(|h| h.0).collect::<Vec<_>>(), [1, 5]);
}

#[test]
fn a_huge_threshold_is_shown_whole_with_the_clause_offset() {
    // Zero boundaries is below 1e20. The limit is 1e20 + 1 clauses, which f64 cannot tell
    // from 1e20, and it must not overflow.
    let rule = metric(
        "clause_boundaries",
        1e20,
        "lt",
        &json!({"scope": "sentence"}),
    );
    let doc = lint(&rule, "One clause only.\n");
    assert_eq!(
        hits(&doc),
        [(1, "Got 1 over 100000000000000000000.".to_owned())]
    );
    let rule = metric(
        "paragraph_sentences",
        1e20,
        "lt",
        &json!({"scope": "paragraph"}),
    );
    let doc = lint(&rule, "One. Two.\n");
    assert_eq!(
        hits(&doc),
        [(1, "Got 2 over 100000000000000000000.".to_owned())]
    );
}
