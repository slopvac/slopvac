//! The mixed-type judgement item.

use std::path::Path;

use slopvac_context::{
    ChecklistEntry, PlainDocument, builtin_genres, checklist_rules, mixed_types_entry,
};
use slopvac_model::{PackId, RuleId};
use slopvac_rules::{CompileOptions, RuleSet};

/// The shipped rules, as the rule crate converts them from the packaged YAML.
fn core() -> RuleSet {
    RuleSet::load_dir(
        &PackId::new("core").unwrap(),
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../slopvac-rules/tests/fixtures/core-v3"),
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("core rules must load:\n{e}"))
}

const RULE_ID: &str = "core:prose-scope.mixed-content-types";

fn entry(path: &str, src: &str) -> Option<ChecklistEntry> {
    let set = core();
    let rule = set.get(&RuleId::new(RULE_ID).unwrap()).unwrap();
    mixed_types_entry(
        &builtin_genres().unwrap(),
        &PlainDocument::from_markdown(path, src),
        rule,
    )
    .unwrap()
}

#[test]
fn loader_supplies_the_rule_with_no_gate() {
    let rules = checklist_rules(&core());
    let rule = rules.iter().find(|r| r.id == RULE_ID).unwrap();
    assert!(rule.gate.only.is_empty() && rule.gate.skip.is_empty());
    assert!(rule.core.is_none());
}

#[test]
fn single_content_type_does_not_fire() {
    assert!(entry("docs/tutorials/a.md", "# A\n\nBody.\n").is_none());
}

#[test]
fn page_without_content_type_does_not_fire() {
    assert!(entry("notes.md", "# Notes\n\nBody.\n").is_none());
}

#[test]
fn tutorial_with_how_to_heading_fires_and_names_both_types() {
    let got = entry(
        "docs/tutorials/deploy.md",
        "# Deploy\n\n## How to deploy\n\nSteps.\n",
    )
    .unwrap();
    assert_eq!(got.id, RULE_ID);
    assert_eq!(
        got.hints,
        vec![
            "mixed content types: split the page so each page serves one reader need (tutorial, how-to)"
        ]
    );
}

#[test]
fn reference_page_id_is_named_when_it_mixes_with_another_type() {
    let got = entry(
        "docs/cli/run.md",
        "# Run\n\n## How to run\n\nSteps.\n\n## Options\n\nFlags.\n",
    )
    .unwrap();
    assert!(got.hints[0].contains("reference-page"), "{:?}", got.hints);
    assert!(got.hints[0].contains("how-to"), "{:?}", got.hints);
}
