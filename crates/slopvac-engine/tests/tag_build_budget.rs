//! A document cannot make the engine build a tag index of unbounded size: the engine charges
//! each tag against a ceiling that follows the document's size before it builds the tag, and
//! ends the run before it builds any index.
//!
//! The tests count the tags built and the elements the index structures are built from, not
//! the time.
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
use slopvac_context::{TagRegistry, TaggerSpec, take_build_elements, take_materialised_tags};
use slopvac_engine::{Engine, EngineError, Input};

/// The ceiling in `docs/contract/policies.md`: one tag for every 64 bytes of the document,
/// plus 65,536.
fn ceiling(source_len: usize) -> usize {
    source_len / 64 + 65_536
}

fn engine() -> Engine {
    common::engine(&[common::tokens("r", "leverage", &json!({}))])
}

/// A paragraph of `sentences` copies of `sentence`.
fn paragraph(sentence: &str, sentences: usize) -> String {
    format!("{}\n", vec![sentence; sentences].join(" "))
}

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

#[test]
fn a_paragraph_of_very_short_requirements_is_not_checked_and_builds_no_tag_past_the_ceiling() {
    // 150000 sentences of 8 bytes: one tag for every 9 bytes, over the one for every 64 the
    // ceiling allows.
    let source = paragraph("It must.", 150_000);
    let limit = ceiling(source.len());
    let _ = (take_build_elements(), take_materialised_tags());
    let result = engine().lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::TagBuildBudget { limit })
    );
    assert_eq!(take_build_elements(), 0, "an index was built");
    // The requirement tagger charges each tag before it builds it, so it builds the limit
    // and no more.
    assert_eq!(take_materialised_tags(), limit);
}

#[test]
fn tags_that_fit_the_ceiling_but_split_into_pieces_past_it_are_not_checked() {
    // 1500 kinds of tag on one cell of 60 lines: 1500 tags, far under the ceiling, that split
    // into 90000 pieces, over the ceiling of about 65,600.
    let (kinds, lines) = (1_500, 60);
    let source = wrapped_table(lines);
    let limit = ceiling(source.len());
    let engine = engine()
        .with_rst(wrapped_rst(lines))
        .with_taggers(taggers(kinds));
    let _ = (take_build_elements(), take_materialised_tags());
    let result = engine.lint(&Input::rst("t.rst", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::TagBuildBudget { limit })
    );
    assert_eq!(take_build_elements(), 0, "an index was built");
    // The 1500 tags, then pieces until the next one would pass the ceiling.
    assert_eq!(take_materialised_tags(), kinds + limit);
}

#[test]
fn a_normal_document_is_checked_and_builds_its_index_once() {
    // 2000 sentences of 68 bytes, each a requirement: far under the ceiling of 100 thousand.
    let sentence = "The service must reject every unsigned request before it parses one.";
    let source = paragraph(sentence, 2_000);
    let _ = take_build_elements();
    let doc = engine()
        .lint(&Input::markdown("a.md", &source))
        .expect("a normal document is checked");
    assert_eq!(doc.findings.len(), 0);
    // Each sentence is a requirement and an enforced invariant: 4000 tags. One structure over
    // them counts 4000; a second index over the same tags would count 8000.
    assert_eq!(take_build_elements(), 4_000);
}

#[test]
fn a_paragraph_of_one_character_sentences_is_not_checked_before_its_sentences_are_all_built() {
    // 300000 sentences of 4 bytes, none of which any tagger tags. One sentence for every 8
    // bytes, plus 65,536, allows 150,000 + 65,536 = 215,536 of them, so the split stops there
    // and the run ends before it holds the rest.
    let source = paragraph("Go.", 300_000);
    let limit = source.len() / 8 + 65_536;
    let result = engine().lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::SentenceBudget { limit })
    );
}

/// A table of `rows` rows of `columns` cells of one character, with its header.
fn table(rows: usize, columns: usize) -> String {
    let row = format!("{}|\n", "|a".repeat(columns));
    let rule = format!("{}|\n", "|-".repeat(columns));
    format!("{row}{rule}{}", row.repeat(rows))
}

#[test]
fn a_table_of_cells_of_one_character_is_not_checked_and_builds_no_index() {
    // 3000 rows of 100 cells of 2 bytes each: 300000 cells in about 603000 bytes, one for
    // every 2 bytes, over the one for every 4 that the cell budget allows.
    let source = table(3_000, 100);
    let limit = source.len() / 4 + 65_536;
    let _ = (take_build_elements(), take_materialised_tags());
    let _ = slopvac_text::take_cell_records_built();
    let result = engine().lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::CellBudget { limit })
    );
    assert_eq!(take_build_elements(), 0, "an index was built");
    // The cells are charged before their records are built, so no record passes the budget.
    let built = slopvac_text::take_cell_records_built();
    assert!(built <= limit, "{built} records for a budget of {limit}");
}

#[test]
fn a_table_with_one_cell_in_every_four_bytes_is_checked() {
    // 300000 rows of one cell: `|a|` and a newline, one cell for every 4 bytes.
    let source = table(300_000, 1);
    let result = engine().lint(&Input::markdown("a.md", &source));
    assert!(result.is_ok(), "{:?}", result.map(|d| d.findings.len()));
}

/// An engine whose one rule reads the whole document as one text.
fn document_engine() -> Engine {
    common::engine(&[common::tokens(
        "r",
        "leverage",
        &json!({"scope": "document"}),
    )])
}

#[test]
fn a_table_cell_of_many_sentences_is_not_checked_when_a_document_rule_splits_it() {
    // One cell holds 300000 sentences of 4 bytes. The cell is one sentence under the cell
    // budget, but a document rule reads the table as one text and splits the cell into
    // sentences. One sentence for every 8 bytes, plus 65,536, allows 215,536 of them.
    let source = format!("| h |\n|---|\n| {} |\n", "Go. ".repeat(300_000));
    let limit = source.len() / 8 + 65_536;
    let result = document_engine().lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::SentenceBudget { limit })
    );
}

/// A tagger that reads the sentences of code blocks and matches none.
fn code_sentence_tagger() -> Result<TagRegistry, Box<dyn std::error::Error>> {
    let yaml = "schema_version: 3\nid: c\ntag: k\nscope: sentence\nwhen:\n  block_kind: code-block\n  sentence: 'zzz'\n";
    let tagger = TaggerSpec::from_yaml(yaml)?.compile()?;
    let mut registry = TagRegistry::new();
    registry.register(Box::new(tagger))?;
    Ok(registry)
}

#[test]
fn a_code_block_of_many_sentences_is_not_checked_when_a_tagger_reads_its_sentences()
-> Result<(), Box<dyn std::error::Error>> {
    // 300000 sentences of 3 bytes in a fenced code block, and a tagger that matches none.
    let source = format!("```\n{}\n```\n", "a. ".repeat(300_000));
    let limit = source.len() / 8 + 65_536;
    let result = engine()
        .with_taggers(code_sentence_tagger()?)
        .lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::SentenceBudget { limit })
    );
    Ok(())
}

#[test]
fn the_split_of_a_dense_paragraph_builds_no_record_past_the_budget() {
    // 300000 sentences of 4 bytes against a budget of 215,536. A split that stops at the
    // budget builds that many records; one that builds them all and then checks builds
    // 300000.
    let source = paragraph("Go.", 300_000);
    let limit = source.len() / 8 + 65_536;
    let _ = slopvac_text::take_sentences_built();
    assert!(engine().lint(&Input::markdown("a.md", &source)).is_err());
    let built = slopvac_text::take_sentences_built();
    assert!(built <= limit, "{built} records for a budget of {limit}");
}

#[test]
fn the_split_of_a_dense_code_block_builds_no_record_past_the_budget()
-> Result<(), Box<dyn std::error::Error>> {
    let source = format!("```\n{}\n```\n", "a. ".repeat(300_000));
    let limit = source.len() / 8 + 65_536;
    let _ = slopvac_context::take_sentences_built();
    let result = engine()
        .with_taggers(code_sentence_tagger()?)
        .lint(&Input::markdown("a.md", &source));
    assert!(result.is_err());
    let built = slopvac_context::take_sentences_built();
    assert!(built <= limit, "{built} records for a budget of {limit}");
    Ok(())
}

#[test]
fn the_split_of_a_composite_text_builds_no_record_past_the_budget() {
    let source = format!("| h |\n|---|\n| {} |\n", "Go. ".repeat(300_000));
    let limit = source.len() / 8 + 65_536;
    let _ = slopvac_text::take_sentences_built();
    assert!(
        document_engine()
            .lint(&Input::markdown("a.md", &source))
            .is_err()
    );
    let built = slopvac_text::take_sentences_built();
    assert!(built <= limit, "{built} records for a budget of {limit}");
}

#[test]
fn the_split_of_a_dense_paragraph_finds_no_boundary_past_the_budget() {
    // The splitter finds the boundaries of a chunk as it goes and stops at the budget. One
    // that finds every boundary of the chunk first, and checks afterwards, finds 300000.
    let source = paragraph("Go.", 300_000);
    let limit = source.len() / 8 + 65_536;
    let _ = slopvac_text::take_cuts_found();
    assert!(engine().lint(&Input::markdown("a.md", &source)).is_err());
    let found = slopvac_text::take_cuts_found();
    assert!(
        found <= limit + 1,
        "{found} boundaries for a budget of {limit}"
    );
}

/// An engine whose one rule matches `; it does not read` and drops the match when
/// `negated-antecedent` finds the negated word in the two sentences before it.
fn antecedent_engine(extra: &serde_json::Value) -> Engine {
    let mut rule = json!({"exceptions": [{"name": "n", "detector": "negated-antecedent"}]});
    if let Some(map) = rule.as_object_mut() {
        map.extend(extra.as_object().cloned().unwrap_or_default());
    }
    common::engine(&[common::rule(
        "r",
        &json!({"kind": "pattern", "pattern": "; it does not read"}),
        &rule,
    )])
}

/// The records the antecedent look-back holds: two sentences, whatever the text before.
const ANTECEDENT_SENTENCES: usize = 2;

#[test]
fn the_antecedent_look_back_in_a_table_cell_holds_only_the_sentences_it_needs() {
    // One cell of 300000 sentences, then a match. Preparation reads the cell as one
    // sentence, so the look-back is the only split of its text.
    let source = format!(
        "| h |\n|---|\n| {}It reads JSON; it does not read YAML. |\n",
        "Go. ".repeat(300_000)
    );
    let _ = slopvac_text::take_peak_sentences_held();
    let result = antecedent_engine(&json!({})).lint(&Input::markdown("a.md", &source));
    assert!(result.is_ok(), "{:?}", result.map(|d| d.findings.len()));
    let held = slopvac_text::take_peak_sentences_held();
    assert!(held <= ANTECEDENT_SENTENCES, "{held} records held at once");
}

#[test]
fn the_antecedent_look_back_in_a_raw_scope_line_holds_only_the_sentences_it_needs() {
    // A raw-scope rule reads the lines of the file, and preparation splits none of a code
    // block: the line of 300000 sentences is split only by the look-back.
    let source = format!(
        "```\n{}It reads JSON; it does not read YAML.\n```\n",
        "Go. ".repeat(300_000)
    );
    let _ = slopvac_text::take_peak_sentences_held();
    let result =
        antecedent_engine(&json!({"scope": "raw"})).lint(&Input::markdown("a.md", &source));
    assert!(result.is_ok(), "{:?}", result.map(|d| d.findings.len()));
    let held = slopvac_text::take_peak_sentences_held();
    assert!(held <= ANTECEDENT_SENTENCES, "{held} records held at once");
}

#[test]
fn a_composite_text_draws_on_the_budget_the_prose_already_used() {
    // 60000 sentences in a paragraph and 60000 in a table cell. The 120000 of the composite
    // text a document rule reads fit the budget of 125,536 on their own, but the paragraph
    // already holds 60000 of it, and 65,536 are left.
    let source = format!(
        "{}\n\n| h |\n|---|\n| {} |\n",
        "Go. ".repeat(60_000),
        "Go. ".repeat(60_000)
    );
    let limit = source.len() / 8 + 65_536;
    assert!(120_000 <= limit, "each split fits on its own");
    let result = document_engine().lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::SentenceBudget { limit })
    );
}

#[test]
fn a_code_block_draws_on_the_budget_the_prose_already_used()
-> Result<(), Box<dyn std::error::Error>> {
    // 100000 sentences in a paragraph and 100000 in a code block a tagger reads. The 100000
    // of the block fit the budget of 165,536 on their own, but the paragraph already holds
    // 100000 of it, and 65,536 are left.
    let source = format!(
        "{}\n\n```\n{}\n```\n",
        "Go. ".repeat(100_000),
        "Go. ".repeat(100_000)
    );
    let limit = source.len() / 8 + 65_536;
    assert!(100_000 <= limit, "each split fits on its own");
    let result = engine()
        .with_taggers(code_sentence_tagger()?)
        .lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::SentenceBudget { limit })
    );
    Ok(())
}

#[test]
fn a_budget_failure_in_a_code_block_wins_over_a_later_tagger_that_fails()
-> Result<(), Box<dyn std::error::Error>> {
    // The first tagger reads the 300000 sentences of the code block and hits the budget. The
    // second hits the PCRE2 match limit on a heading. The run ends on the budget, which is the
    // first failure, not on the later one.
    let mut registry = code_sentence_tagger()?;
    let yaml = "schema_version: 3\nid: z\ntag: k\nscope: paragraph\nwhen:\n  block_kind: paragraph\n  heading_path: '^(a|aa)+$'\n";
    registry.register(Box::new(TaggerSpec::from_yaml(yaml)?.compile()?))?;
    let source = format!(
        "```\n{}\n```\n\n# {}b\n\nText.\n",
        "Go. ".repeat(300_000),
        "a".repeat(64)
    );
    let limit = source.len() / 8 + 65_536;
    let result = engine()
        .with_taggers(registry)
        .lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::SentenceBudget { limit })
    );
    Ok(())
}

#[test]
fn the_antecedent_look_back_asks_the_earlier_block_only_for_the_records_it_is_missing() {
    // The match sits in a sentence whose text before it, `It reads JSON`, is one sentence, so
    // one of the two antecedents is missing. The table cell before it holds 300000 sentences,
    // which preparation reads as one: the cell is asked for one record, so at most one is
    // alive at once.
    let source = format!(
        "| h |\n|---|\n| {} |\n\nIt reads JSON; it does not read YAML.\n",
        "Go. ".repeat(300_000)
    );
    let _ = slopvac_text::take_peak_sentences_held();
    let result = antecedent_engine(&json!({})).lint(&Input::markdown("a.md", &source));
    assert!(result.is_ok(), "{:?}", result.map(|d| d.findings.len()));
    let held = slopvac_text::take_peak_sentences_held();
    assert_eq!(held, 1, "{held} records alive at once");
}

#[test]
fn code_blocks_that_together_pass_the_budget_are_not_checked_when_a_tagger_reads_them()
-> Result<(), Box<dyn std::error::Error>> {
    // Three fenced code blocks of 100000 sentences of 3 bytes each, and a tagger that matches
    // none. The text is about 900,000 bytes, so the budget is 900,000 / 8 + 65,536, about
    // 178,000. Each block fits it on its own, but the three together hold 300000.
    let block = format!("```\n{}\n```\n\n", "a. ".repeat(100_000));
    let source = block.repeat(3);
    let limit = source.len() / 8 + 65_536;
    assert!(100_000 <= limit, "each block fits on its own");
    assert!(300_000 > limit, "the three together do not");
    let result = engine()
        .with_taggers(code_sentence_tagger()?)
        .lint(&Input::markdown("a.md", &source));
    assert_eq!(
        result.map(|d| d.findings.len()),
        Err(EngineError::SentenceBudget { limit })
    );
    Ok(())
}
