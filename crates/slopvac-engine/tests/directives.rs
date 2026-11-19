//! Inline directives.

#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

mod common;

use common::{category, engine, places, ruleset, settings, tokens};
use serde_json::json;
use slopvac_engine::{Engine, Input, Level};
use slopvac_model::{Document, Severity};

fn dull() -> serde_json::Value {
    tokens(
        "dull",
        "leverage",
        &json!({"exceptions": [{"name": "dead-metaphor"}]}),
    )
}

fn lint(source: &str) -> Document {
    engine(&[dull()])
        .lint(&Input::markdown("a.md", source))
        .unwrap()
}

fn rules(doc: &Document) -> Vec<(&str, usize)> {
    doc.findings
        .iter()
        .map(|f| (f.rule_id.as_str(), f.start.line()))
        .collect()
}

const ALLOW: &str = "<!-- slopvac-allow: rule=core:cat.dull reason=dead-metaphor -->";

#[test]
fn allow_with_a_listed_reason_suppresses_the_next_block() {
    let doc = lint(&format!(
        "{ALLOW}\nPlease leverage it,\nand leverage it again.\n"
    ));
    assert!(doc.findings.is_empty());
}

#[test]
fn allow_reaches_over_blank_lines_but_not_past_the_next_block() {
    let doc = lint(&format!(
        "{ALLOW}\n\nPlease leverage it.\n\nNext leverage.\n"
    ));
    assert_eq!(rules(&doc), [("core:cat.dull", 5)]);
}

#[test]
fn allow_with_an_unlisted_reason_is_reported_and_does_not_suppress() {
    let doc =
        lint("<!-- slopvac-allow: rule=core:cat.dull reason=bogus -->\nPlease leverage it.\n");
    assert_eq!(
        rules(&doc),
        [("core:meta.invalid-suppression", 1), ("core:cat.dull", 2)]
    );
    assert_eq!(
        doc.findings[0].message.as_str(),
        "reason \"bogus\" is not an exception of core:cat.dull. Valid: dead-metaphor."
    );
    assert_eq!(doc.findings[0].severity, Severity::Error);
    assert_eq!(doc.findings[0].category.as_str(), "meta");
}

#[test]
fn allow_with_no_reason_is_reported_and_does_not_suppress() {
    let doc = lint("<!-- slopvac-allow: rule=core:cat.dull -->\nPlease leverage it.\n");
    assert_eq!(
        doc.findings[0].message.as_str(),
        "suppression of core:cat.dull names no reason. Add reason=<one of: dead-metaphor>."
    );
    assert_eq!(doc.findings.len(), 2);
}

#[test]
fn a_2x_rule_id_points_at_migrate_and_does_not_suppress() {
    let doc =
        lint("<!-- slopvac-allow: rule=cat.dull reason=dead-metaphor -->\nPlease leverage it.\n");
    assert_eq!(
        doc.findings[0].message.as_str(),
        "suppression names cat.dull, a 2.x rule id with no pack. Run `slopvac migrate` to \
         qualify it as <plugin>:cat.dull."
    );
    assert_eq!(doc.findings.len(), 2);
}

#[test]
fn allow_for_an_unknown_rule_is_reported() {
    let doc = lint("<!-- slopvac-allow: rule=core:cat.nope reason=x -->\nPlain text.\n");
    assert_eq!(
        doc.findings[0].message.as_str(),
        "suppression names unknown rule core:cat.nope."
    );
}

#[test]
fn a_malformed_allow_is_reported_with_the_expected_form() {
    let doc = lint("<!-- slopvac-allow: core:cat.dull -->\nPlain text.\n");
    assert_eq!(
        doc.findings[0].message.as_str(),
        "invalid suppression; expected <!-- slopvac-allow: rule=<plugin>:<rule-id> \
         reason=<reason> -->."
    );
}

#[test]
fn allow_for_a_rule_that_is_off_stays_silent() {
    let mut s = settings();
    s.rule_levels
        .insert(common::id("core:cat.dull"), Level::Off);
    let e = Engine::new(vec![ruleset(vec![category("cat", &[dull()])])], s).unwrap();
    let doc = e
        .lint(&Input::markdown(
            "a.md",
            "<!-- slopvac-allow: rule=core:cat.dull reason=whatever -->\nText.\n",
        ))
        .unwrap();
    assert!(doc.findings.is_empty());
}

#[test]
fn disable_next_line_covers_the_following_block_only() {
    let doc = lint("<!-- slopvac-disable-next-line -->\nPlease leverage it.\n\nMore leverage.\n");
    assert_eq!(rules(&doc), [("core:cat.dull", 4)]);
}

#[test]
fn disable_and_enable_bound_a_region() {
    let doc = lint(
        "Early leverage.\n\n<!-- slopvac-disable -->\n\nHidden leverage.\n\n<!-- slopvac-enable -->\n\nLate leverage.\n",
    );
    assert_eq!(rules(&doc), [("core:cat.dull", 1), ("core:cat.dull", 9)]);
}

#[test]
fn a_directive_in_a_code_fence_or_code_span_is_an_example() {
    let doc = lint(
        "```md\n<!-- slopvac-disable -->\n<!-- slopvac-allow: rule=bad -->\n```\n\nWrite `<!-- slopvac-disable -->` to stop.\n\nPlease leverage it.\n",
    );
    assert_eq!(rules(&doc), [("core:cat.dull", 8)]);
}

#[test]
fn a_directive_in_front_matter_is_an_example() {
    let doc = lint("---\nnote: <!-- slopvac-disable -->\n---\n\nPlease leverage it.\n");
    assert_eq!(rules(&doc), [("core:cat.dull", 5)]);
}

#[test]
fn an_allow_before_a_table_covers_every_row() {
    let doc = lint(&format!(
        "{ALLOW}\n| a | b |\n|---|---|\n| leverage | x |\n| y | leverage |\n\nAfter leverage.\n"
    ));
    assert_eq!(places(&doc).len(), 1);
    assert_eq!(doc.findings[0].start.line(), 7);
}
