//! `rules-output.schema.json` writes the category id in place, because `lint-report.schema.json`
//! publishes `$defs/CategoryId` and a definition name has one owner. Nothing links the two
//! copies, so this test holds them equal.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use serde_json::Value;
use slopvac_schemas::{generate, to_canonical_json};

fn generated(file: &str) -> Value {
    let files = generate().unwrap();
    let found = files.iter().find(|g| g.file == file).unwrap();
    serde_json::from_str(&found.text).unwrap()
}

#[test]
fn the_category_id_in_the_rules_output_is_the_one_the_lint_report_publishes() {
    let published = to_canonical_json(&generated("lint-report.schema.json")["$defs"]["CategoryId"]);
    let output = generated("rules-output.schema.json");
    let listing = &output["$defs"]["CategoryOutput"]["properties"]["id"];
    assert_eq!(to_canonical_json(listing), published);

    // On a rule entry the written-out definition sits beside the member's own description.
    let mut rule = output["$defs"]["RuleOutput"]["properties"]["category"].clone();
    rule.as_object_mut().unwrap().remove("description");
    assert_eq!(to_canonical_json(&rule), published);
}
