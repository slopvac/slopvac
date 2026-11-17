//! Checklist filtering: remainders follow their core rule, unused contexts drop rules.

use slopvac_context::{
    ChecklistRule, ContextGate, checklist, context_hint, effective_gates, rule_allows,
};

fn gate(only: &[&str], skip: &[&str]) -> ContextGate {
    ContextGate {
        only: only.iter().map(|s| (*s).into()).collect(),
        skip: skip.iter().map(|s| (*s).into()).collect(),
    }
}

fn rule(id: &str, g: ContextGate, core: Option<&str>, hints: &[&str]) -> ChecklistRule {
    ChecklistRule {
        id: id.into(),
        gate: g,
        core: core.map(Into::into),
        hint_contexts: hints.iter().map(|s| (*s).into()).collect(),
    }
}

fn rules() -> Vec<ChecklistRule> {
    vec![
        rule(
            "prose-scope.history-narration",
            gate(&[], &["change-note"]),
            None,
            &[],
        ),
        rule(
            "prose-scope.history-narration-remainder",
            ContextGate::default(),
            Some("prose-scope.history-narration"),
            &[],
        ),
        rule(
            "prose-scope.negative-guarantee-remainder",
            ContextGate::default(),
            None,
            &["security"],
        ),
        rule(
            "prose-craft.procedure-only",
            gate(&["procedure"], &[]),
            None,
            &[],
        ),
    ]
}

fn ids(entries: &[slopvac_context::ChecklistEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.id.as_str()).collect()
}

#[test]
fn remainder_inherits_the_core_rules_exemption() {
    let all = rules();
    assert_eq!(
        effective_gates(&all[1], &all),
        vec![ContextGate::default(), gate(&[], &["change-note"])]
    );
    assert_eq!(effective_gates(&all[0], &all), vec![all[0].gate.clone()]);
    assert!(rule_allows(&all[1], &all, &["requirement"]));
    assert!(!rule_allows(&all[1], &all, &["change-note"]));
}

#[test]
fn remainder_and_core_only_lists_are_evaluated_separately_and_anded() {
    let all = vec![
        rule("core", gate(&["a", "b"], &[]), None, &[]),
        rule("rem", gate(&["b", "c"], &[]), Some("core"), &[]),
    ];
    let cases: [(&[&str], bool); 6] = [
        (&["a"], false),
        (&["b"], true),
        (&["c"], false),
        (&["a", "c"], true),
        (&["d"], false),
        (&[], false),
    ];
    for (kinds, want) in cases {
        assert_eq!(rule_allows(&all[1], &all, kinds), want, "allows {kinds:?}");
        let kept = checklist(&all, kinds, &[]);
        let has_rem = kept.iter().any(|e| e.id == "rem");
        assert_eq!(has_rem, want, "checklist {kinds:?}");
    }
}

#[test]
fn release_notes_checklist_omits_the_history_rule_and_its_remainder() {
    let got = checklist(&rules(), &["change-note"], &["change-note"]);
    assert_eq!(ids(&got), vec!["prose-scope.negative-guarantee-remainder"]);
}

#[test]
fn partial_change_note_context_keeps_the_history_rules() {
    let got = checklist(&rules(), &["change-note"], &[]);
    assert!(ids(&got).contains(&"prose-scope.history-narration"));
    assert!(ids(&got).contains(&"prose-scope.history-narration-remainder"));
}

#[test]
fn rules_that_need_an_absent_context_are_dropped() {
    assert!(!ids(&checklist(&rules(), &[], &[])).contains(&"prose-craft.procedure-only"));
    assert!(ids(&checklist(&rules(), &["procedure"], &[])).contains(&"prose-craft.procedure-only"));
}

#[test]
fn security_hint_lands_only_in_entries_that_ask_for_it() {
    let got = checklist(&rules(), &["security"], &[]);
    let entry = got
        .iter()
        .find(|e| e.id == "prose-scope.negative-guarantee-remainder")
        .unwrap();
    assert_eq!(
        entry.hints,
        vec![context_hint("security").unwrap().to_owned()]
    );
    assert!(
        got.iter()
            .filter(|e| e.id != entry.id)
            .all(|e| e.hints.is_empty())
    );
    let none = checklist(&rules(), &[], &[]);
    assert!(none.iter().all(|e| e.hints.is_empty()));
}
