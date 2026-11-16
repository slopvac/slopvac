//! The mixed-type judgement item.

use slopvac_context::{
    ChecklistEntry, MIXED_TYPES_RULE_ID, PlainDocument, builtin_genres, mixed_types_entry,
};

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn entry(path: &str, src: &str) -> Option<ChecklistEntry> {
    mixed_types_entry(
        &builtin_genres().unwrap(),
        &PlainDocument::from_markdown(path, src),
    )
    .unwrap()
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
    assert_eq!(got.id, MIXED_TYPES_RULE_ID);
    assert_eq!(
        got.hints,
        vec![
            "This page mixes content types (tutorial, how-to). Split it so each page serves one reader need."
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
