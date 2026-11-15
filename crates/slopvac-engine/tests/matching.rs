//! Matching: scopes, position mapping, filters and exceptions.

#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs,
    clippy::single_range_in_vec_init
)]

mod common;

use common::{engine, id, matched, places, rule, tokens};
use serde_json::json;
use slopvac_engine::Input;
use slopvac_model::Severity;

fn lint(engine: &slopvac_engine::Engine, source: &str) -> slopvac_model::Document {
    engine.lint(&Input::markdown("a.md", source)).unwrap()
}

#[test]
fn a_match_reports_its_position_span_severity_and_message() {
    let e = engine(&[tokens("dull", "leverage", &json!({}))]);
    let doc = lint(&e, "Please leverage the tool.\n");
    assert_eq!(doc.findings.len(), 1);
    let f = &doc.findings[0];
    assert_eq!(f.rule_id, id("core:cat.dull"));
    assert_eq!(f.pack.as_str(), "core");
    assert_eq!(f.category.as_str(), "cat");
    assert_eq!((f.start.line(), f.start.column()), (1, 8));
    let extent = f.extent.unwrap();
    assert_eq!((extent.span.start(), extent.span.end()), (7, 15));
    assert_eq!((extent.end.line(), extent.end.column()), (1, 16));
    assert_eq!(f.severity, Severity::Warning);
    assert_eq!(f.message.as_str(), "Found leverage.");
    assert_eq!(f.matched_text, "leverage");
}

#[test]
fn a_match_inside_markup_maps_to_the_source_bytes() {
    let e = engine(&[tokens("dull", "leverage", &json!({}))]);
    let doc = lint(&e, "Use **leverage** now.\n");
    let span = doc.findings[0].extent.unwrap().span;
    assert_eq!((span.start(), span.end()), (6, 14));
}

#[test]
fn inline_code_is_not_prose() {
    let e = engine(&[tokens("dull", "leverage", &json!({}))]);
    assert!(lint(&e, "Call `leverage` here.\n").findings.is_empty());
}

#[test]
fn front_matter_is_never_linted_and_lines_stay_absolute() {
    let e = engine(&[tokens("dull", "leverage", &json!({}))]);
    let doc = lint(&e, "---\ntitle: leverage\n---\n\nPlease leverage it.\n");
    assert_eq!(places(&doc), [("core:cat.dull".to_owned(), 5, 8)]);
}

#[test]
fn a_raw_rule_reads_front_matter() {
    let e = engine(&[rule(
        "raw",
        &json!({"kind": "tokens", "tokens": ["leverage"]}),
        &json!({"scope": "raw"}),
    )]);
    let doc = lint(&e, "---\ntitle: leverage\n---\n\nBody.\n");
    assert_eq!(places(&doc), [("core:cat.raw".to_owned(), 2, 8)]);
}

#[test]
fn a_heading_rule_reads_only_headings() {
    let e = engine(&[rule(
        "head",
        &json!({"kind": "tokens", "tokens": ["overview"]}),
        &json!({"scope": "heading"}),
    )]);
    let doc = lint(&e, "# Overview\n\nAn overview of it.\n");
    assert_eq!(places(&doc), [("core:cat.head".to_owned(), 1, 3)]);
}

#[test]
fn a_sentence_rule_matches_inside_one_sentence() {
    // The pattern needs both words in one sentence. The words in two sentences do not match.
    let e = engine(&[rule(
        "pair",
        &json!({"kind": "pattern", "pattern": r"^Fast\b.*slow$"}),
        &json!({"scope": "sentence"}),
    )]);
    assert_eq!(lint(&e, "Fast and slow. Other text.\n").findings.len(), 0);
    assert_eq!(lint(&e, "Fast and slow\n").findings.len(), 1);
}

#[test]
fn a_document_rule_sees_text_across_blocks() {
    let e = engine(&[rule(
        "across",
        &json!({"kind": "pattern", "pattern": r"first one\. Second"}),
        &json!({"scope": "document"}),
    )]);
    let doc = lint(&e, "The first one.\n\nSecond block.\n");
    assert_eq!(doc.findings.len(), 1);
    let span = doc.findings[0].extent.unwrap().span;
    assert_eq!((span.start(), span.end()), (4, 22));
}

#[test]
fn text_type_limits_a_rule_to_matching_sentences() {
    let e = engine(&[rule(
        "proc",
        &json!({"kind": "tokens", "tokens": ["quickly"]}),
        &json!({"text_type": "procedural"}),
    )]);
    let doc = lint(
        &e,
        "Open the valve quickly.\n\nThe valve opens quickly in the summer heat.\n",
    );
    assert_eq!(matched(&doc), ["quickly"]);
    assert_eq!(doc.findings[0].start.line(), 1);
}

#[test]
fn an_allowlist_entry_matches_without_regard_to_case() {
    let e = engine(&[rule(
        "dull",
        &json!({"kind": "tokens", "tokens": ["leverage"], "ignore_case": true}),
        &json!({"allowlist": ["Leverage"]}),
    )]);
    assert!(lint(&e, "A leverage point.\n").findings.is_empty());
}

#[test]
fn a_match_inside_an_allowlisted_phrase_is_skipped() {
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"allowlist": ["financial leverage ratio"]}),
    )]);
    let doc = lint(&e, "The financial leverage ratio and a leverage point.\n");
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(doc.findings[0].start.column(), 36);
}

#[test]
fn a_match_in_capitals_is_exempt_unless_the_rule_asks() {
    let e = engine(&[
        rule(
            "quiet",
            &json!({"kind": "tokens", "tokens": ["ensure"], "ignore_case": true}),
            &json!({}),
        ),
        rule(
            "loud",
            &json!({"kind": "tokens", "tokens": ["ensure"], "ignore_case": true, "match_all_caps": true}),
            &json!({}),
        ),
    ]);
    let doc = lint(&e, "ENSURE the build.\n");
    assert_eq!(places(&doc), [("core:cat.loud".to_owned(), 1, 1)]);
}

#[test]
fn a_substitution_names_its_replacement_and_offers_one_edit() {
    let e = engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {"utilize": "use", "e\\.g\\.(?=,)": "for example"}}),
        &json!({"message": "Use {replacement} for {match}."}),
    )]);
    let doc = lint(&e, "We utilize it, e.g., here.\n");
    assert_eq!(doc.findings.len(), 2);
    assert_eq!(doc.findings[0].message.as_str(), "Use use for utilize.");
    let edit = &doc.findings[0].fix.as_ref().unwrap().edits()[0];
    assert_eq!(
        (edit.span.start(), edit.span.end(), edit.text.as_str()),
        (3, 10, "use")
    );
    assert_eq!(
        doc.findings[1].message.as_str(),
        "Use for example for e.g.."
    );
}

#[test]
fn a_replacement_with_alternatives_offers_no_edit() {
    let e = engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {"via": "by, or through"}}),
        &json!({}),
    )]);
    let doc = lint(&e, "Send it via mail.\n");
    assert_eq!(doc.findings.len(), 1);
    assert!(doc.findings[0].fix.is_none());
}

#[test]
fn a_rule_that_excludes_headings_is_silent_in_them() {
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"exclude_blocks": ["heading"]}),
    )]);
    let doc = lint(&e, "# On leverage\n\nSome leverage.\n");
    assert_eq!(places(&doc), [("core:cat.dull".to_owned(), 3, 6)]);
}

#[test]
fn the_quotation_detector_drops_a_quoted_match() {
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"exceptions": [{"name": "quotation", "detector": "quotation"}]}),
    )]);
    let doc = lint(&e, "Never say \"leverage\" here. Do leverage it.\n");
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(doc.findings[0].start.column(), 31);
}

#[test]
fn an_exception_without_a_detector_does_not_drop_anything() {
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"exceptions": [{"name": "quotation"}]}),
    )]);
    assert_eq!(lint(&e, "Never say \"leverage\".\n").findings.len(), 1);
}

#[test]
fn a_context_tag_named_as_a_detector_drops_findings_it_covers() {
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"exceptions": [{"name": "requirement", "detector": "requirement"}]}),
    )]);
    let doc = lint(
        &e,
        "You must leverage the cache.\n\nWe leverage the cache.\n",
    );
    assert_eq!(places(&doc), [("core:cat.dull".to_owned(), 3, 4)]);
}

#[test]
fn a_context_gate_skips_and_selects_by_tag() {
    let skip = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"contexts": {"skip": ["requirement"]}}),
    )]);
    let only = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"contexts": {"only": ["requirement"]}}),
    )]);
    let text = "You must leverage the cache.\n\nWe leverage the cache.\n";
    assert_eq!(
        places(&lint(&skip, text)),
        [("core:cat.dull".to_owned(), 3, 4)]
    );
    assert_eq!(
        places(&lint(&only, text)),
        [("core:cat.dull".to_owned(), 1, 10)]
    );
}

#[test]
fn a_finding_lists_the_context_tags_that_cover_it() {
    let e = engine(&[tokens("dull", "leverage", &json!({}))]);
    let doc = lint(&e, "You must leverage the cache.\n");
    let tags: Vec<&str> = doc.findings[0]
        .contexts
        .iter()
        .map(slopvac_model::ContextTag::as_str)
        .collect();
    assert!(tags.contains(&"requirement"), "{tags:?}");
    assert!(
        doc.contexts_seen
            .iter()
            .any(|t| t.as_str() == "requirement")
    );
}

#[test]
fn findings_sort_by_position_then_rule_id() {
    let e = engine(&[
        tokens("zeta", "leverage", &json!({})),
        tokens("alpha", "leverage", &json!({})),
        tokens("early", "Please", &json!({})),
    ]);
    let doc = lint(&e, "Please leverage it.\n");
    let order: Vec<_> = places(&doc).into_iter().map(|p| p.0).collect();
    assert_eq!(order, ["core:cat.early", "core:cat.alpha", "core:cat.zeta"]);
}

#[test]
fn crlf_line_endings_keep_byte_exact_columns() {
    let e = engine(&[tokens("dull", "leverage", &json!({}))]);
    let doc = lint(&e, "One.\r\n\r\nUse leverage.\r\n");
    assert_eq!(places(&doc), [("core:cat.dull".to_owned(), 3, 5)]);
    let span = doc.findings[0].extent.unwrap().span;
    assert_eq!((span.start(), span.end()), (12, 20));
}

/// The exclusive end of a finding as `(line, column)`, and its byte span.
fn extent(doc: &slopvac_model::Document) -> ((usize, usize), (usize, usize)) {
    let e = doc.findings[0].extent.unwrap();
    (
        (e.end.line(), e.end.column()),
        (e.span.start(), e.span.end()),
    )
}

#[test]
fn a_match_across_blocks_reports_its_exclusive_end_line_and_column() {
    let e = engine(&[rule(
        "across",
        &json!({"kind": "pattern", "pattern": r"first one\. Second"}),
        &json!({"scope": "document"}),
    )]);
    // The match ends after "Second" on line 3, so the end column is 7 (code points).
    assert_eq!(
        extent(&lint(&e, "The first one.\n\nSecond block.\n")),
        ((3, 7), (4, 22))
    );
    // CRLF: two bytes per break, and the end is still line 3, column 7.
    assert_eq!(
        extent(&lint(&e, "The first one.\r\n\r\nSecond block.\r\n")),
        ((3, 7), (4, 24))
    );
}

#[test]
fn an_end_column_counts_code_points_not_bytes() {
    let e = engine(&[rule(
        "across",
        &json!({"kind": "pattern", "pattern": r"first one\. S.cond"}),
        &json!({"scope": "document"}),
    )]);
    // "Ünï " is 6 bytes for 4 characters, and the "é" of line 3 takes two bytes.
    let doc = lint(&e, "Ünï first one.\n\nSécond block.\n");
    assert_eq!(
        extent(&doc),
        ((3, 7), (6, 6 + "first one.\n\nSécond".len()))
    );
}

#[test]
fn a_rule_that_excludes_list_items_is_silent_in_them() {
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"exclude_blocks": ["list-item"]}),
    )]);
    let doc = lint(
        &e,
        "- Use leverage.\n  1. Nested leverage.\n\nOutside leverage.\n",
    );
    assert_eq!(places(&doc), [("core:cat.dull".to_owned(), 4, 9)]);
}

#[test]
fn a_rule_that_excludes_quotes_is_silent_in_them() {
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"exclude_blocks": ["quote"]}),
    )]);
    let doc = lint(&e, "> Quoted leverage.\n\nOutside leverage.\n");
    assert_eq!(places(&doc), [("core:cat.dull".to_owned(), 3, 9)]);
}

#[test]
fn the_quote_context_covers_quoted_blocks() {
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"contexts": {"skip": ["quote"]}}),
    )]);
    let doc = lint(&e, "> Quoted leverage.\n\nOutside leverage.\n");
    assert_eq!(places(&doc), [("core:cat.dull".to_owned(), 3, 9)]);
}

#[test]
fn list_and_quote_text_is_not_counted_as_paragraphs() {
    let e = engine(&[tokens("dull", "leverage", &json!({}))]);
    let doc = lint(&e, "One two.\n\n- item one\n\n> quote text\n\nLast.\n");
    assert_eq!(doc.paragraphs, 2);
    assert_eq!(doc.sentences, 4);
}

#[test]
fn an_allowlisted_phrase_still_holds_when_other_text_changes_length_in_lower_case() {
    // U+0130 lower-cases to three bytes, so byte offsets shift. The phrase must still hold.
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"allowlist": ["financial leverage ratio"]}),
    )]);
    let doc = lint(
        &e,
        "\u{130}stanbul: the financial leverage ratio and a leverage point.\n",
    );
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(doc.findings[0].matched_text, "leverage");
    assert!(doc.findings[0].start.column() > 40);
}

#[test]
fn a_list_under_a_version_lead_in_carries_the_change_note_context() {
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"contexts": {"skip": ["change-note"]}}),
    )]);
    let doc = lint(
        &e,
        "In 2.0:\n\n- Use leverage here.\n- More leverage.\n\nOutside leverage.\n",
    );
    assert_eq!(places(&doc), [("core:cat.dull".to_owned(), 6, 9)]);
    assert!(
        doc.contexts_seen
            .iter()
            .any(|t| t.as_str() == "change-note")
    );
}

#[test]
fn a_greek_final_sigma_does_not_defeat_an_allowlisted_phrase() {
    // Whole-string lower case turns the final sigma of "ΟΣ" into "ς" before a space.
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"allowlist": ["ος leverage"]}),
    )]);
    let doc = lint(&e, "Ο ΟΣ leverage here and a leverage there.\n");
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(doc.findings[0].start.column(), 26);
}

#[test]
fn a_pack_rule_that_matches_one_byte_is_refused_when_the_pack_loads() {
    // `\C\K\C` on "é" would give a span that starts inside the character. The pattern layer
    // refuses `\C`, so the pack fails to load with the file and rule named, and no span can
    // reach the code that slices the document.
    let r = rule(
        "bytes",
        &json!({"kind": "pattern", "pattern": r"\C\K\C"}),
        &json!({}),
    );
    let (path, source) = common::category("cat", &[r]);
    let err = slopvac_rules::RuleSet::from_sources(
        &slopvac_model::PackId::new("core").unwrap(),
        vec![(path, source)],
        &slopvac_rules::CompileOptions::default(),
    )
    .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("not allowed"), "{text}");
    assert!(text.contains("cat.json"), "{text}");
}

#[test]
fn a_match_reset_reports_spans_that_are_safe_to_slice() {
    let e = engine(&[rule(
        "reset",
        &json!({"kind": "pattern", "pattern": r"caf\K."}),
        &json!({}),
    )]);
    let doc = lint(&e, "Order a café now.\n");
    assert_eq!(matched(&doc), ["é"]);
}

#[test]
fn a_match_reset_inside_a_lookahead_is_refused_when_the_pack_loads() {
    // PCRE2 10.46 rejects `\K` in a lookaround, the only way to leave a start after the end.
    let r = rule(
        "odd",
        &json!({"kind": "pattern", "pattern": r"(?=é\K)"}),
        &json!({}),
    );
    let (path, source) = common::category("cat", &[r]);
    let err = slopvac_rules::RuleSet::from_sources(
        &slopvac_model::PackId::new("core").unwrap(),
        vec![(path, source)],
        &slopvac_rules::CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("lookarounds"), "{err}");
}
