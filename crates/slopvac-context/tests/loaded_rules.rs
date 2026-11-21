//! The checklist adapter: judgement rules loaded from category files keep their gates, and
//! checked rules stay out.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::path::PathBuf;

use slopvac_context::{ChecklistRule, checklist, checklist_rules};
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, RuleSet};

fn judgement(id: &str, contexts: &str) -> String {
    format!(
        r#"{{"id":"{id}","name":"Name","message":"Message","severity":"suggestion","scope":"document",
        {contexts}
        "match":{{"kind":"judgement","question":"Is it?"}},
        "ai_signal":"none","ai_signal_source":"unmeasured",
        "citation":{{"source":"test"}}}}"#
    )
}

fn checked(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","name":"Name","message":"Message","severity":"warning","scope":"prose",
        "contexts":{{"skip":["quotation"]}},
        "match":{{"kind":"tokens","tokens":["foo"]}},
        "examples":[{{"bad":"foo","good":"bar"}}],
        "ai_signal":"none","ai_signal_source":"unmeasured",
        "citation":{{"source":"test"}}}}"#
    )
}

fn loaded() -> Vec<ChecklistRule> {
    let rules = [
        judgement("plain", ""),
        judgement("needs-steps", r#""contexts":{"only":["procedure"]},"#),
        judgement("not-in-notes", r#""contexts":{"skip":["change-note"]},"#),
        checked("lexical"),
    ]
    .join(",");
    let text = format!(
        r#"{{"schema_version":3,"id":"cat","title":"Cat","description":"D","weight":1.0,
        "recommended_for":["consumer"],"rules":[{rules}]}}"#
    );
    let set = RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        [(PathBuf::from("cat.json"), text)],
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("fixture must load:\n{e}"));
    checklist_rules(&set)
}

fn kept(seen: &[&str], covering: &[&str]) -> Vec<String> {
    checklist(&loaded(), seen, covering)
        .into_iter()
        .map(|e| e.id)
        .collect()
}

#[test]
fn only_judgement_rules_are_adapted() {
    let ids: Vec<String> = loaded().into_iter().map(|r| r.id).collect();
    assert_eq!(
        ids,
        [
            "core:cat.plain",
            "core:cat.needs-steps",
            "core:cat.not-in-notes"
        ]
    );
}

#[test]
fn only_gate_is_kept_and_needs_its_context() {
    let rules = loaded();
    let rule = rules
        .iter()
        .find(|r| r.id.ends_with("needs-steps"))
        .unwrap();
    assert_eq!(rule.gate.only, ["procedure"]);
    assert_eq!(rule.gate.skip, Vec::<String>::new());
    assert!(!kept(&[], &[]).iter().any(|i| i.ends_with("needs-steps")));
    assert!(
        kept(&["procedure"], &[])
            .iter()
            .any(|i| i.ends_with("needs-steps"))
    );
}

#[test]
fn skip_gate_is_kept_and_drops_where_the_context_covers_the_page() {
    let rules = loaded();
    let rule = rules
        .iter()
        .find(|r| r.id.ends_with("not-in-notes"))
        .unwrap();
    assert_eq!(rule.gate.skip, ["change-note"]);
    assert_eq!(rule.gate.only, Vec::<String>::new());
    assert!(kept(&[], &[]).iter().any(|i| i.ends_with("not-in-notes")));
    assert!(
        !kept(&["change-note"], &["change-note"])
            .iter()
            .any(|i| i.ends_with("not-in-notes"))
    );
}
