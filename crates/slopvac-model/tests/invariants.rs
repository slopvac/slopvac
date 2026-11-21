//! Report types must not hold, accept or emit a state the lint-report contract forbids.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

mod common;

use std::fs;
use std::path::Path;

use common::patch_layer;
use serde_json::{Value, json};
use slopvac_model::{
    ConfigSource, ContextTag, Contexts, Edit, Finding, Fix, Gate, Notice, Report, RuleProvenance,
    Span,
};

fn fixture(dir: &str, name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v3/conformance/lint-report")
        .join(dir)
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// Finding with a span (document 0, finding 1 of the multi-document fixture).
fn spanned_finding() -> Value {
    fixture("valid", "multi-document.json")["documents"][0]["findings"][1].clone()
}

fn edit(start: usize, end: usize) -> Edit {
    Edit {
        span: Span::new(start, end).unwrap(),
        text: "x".into(),
    }
}

#[test]
fn gate_rejects_failed_without_reasons() {
    let gate = json!({"passed": false, "reasons": []});
    assert!(serde_json::from_value::<Gate>(gate).is_err());
}

#[test]
fn gate_rejects_passed_with_reasons() {
    let gate = json!({"passed": true, "reasons": ["too dense"]});
    assert!(serde_json::from_value::<Gate>(gate).is_err());
}

#[test]
fn gate_rejects_an_empty_reason() {
    let gate = json!({"passed": false, "reasons": [""]});
    assert!(serde_json::from_value::<Gate>(gate).is_err());
}

#[test]
fn gate_construction_rejects_no_reasons_and_empty_reasons() {
    assert!(Gate::failed(vec![]).is_err());
    assert!(Gate::failed(vec![String::new()]).is_err());
}

#[test]
fn gate_round_trips_both_outcomes() {
    let passed = Gate::passed();
    assert!(passed.is_passed() && passed.reasons().is_empty());
    assert_eq!(
        serde_json::to_value(&passed).unwrap(),
        json!({"passed": true, "reasons": []})
    );
    let failed = Gate::failed(vec!["too dense".into()]).unwrap();
    assert!(!failed.is_passed());
    assert_eq!(
        serde_json::to_value(&failed).unwrap(),
        json!({"passed": false, "reasons": ["too dense"]})
    );
    let back: Gate = serde_json::from_value(serde_json::to_value(&failed).unwrap()).unwrap();
    assert_eq!(back, failed);
}

#[test]
fn finding_rejects_a_partial_span_and_end_position() {
    for (case, mutate) in [
        ("span missing", ("span", None)),
        ("span null", ("span", Some(Value::Null))),
        ("end_line missing", ("end_line", None)),
        ("end_line null", ("end_line", Some(Value::Null))),
        ("end_column missing", ("end_column", None)),
        ("end_column null", ("end_column", Some(Value::Null))),
    ] {
        let mut finding = spanned_finding();
        let object = finding.as_object_mut().unwrap();
        match mutate.1 {
            None => {
                object.remove(mutate.0);
            }
            Some(value) => {
                object.insert(mutate.0.into(), value);
            }
        }
        assert!(
            serde_json::from_value::<Finding>(finding).is_err(),
            "{case} must be rejected"
        );
    }
}

#[test]
fn finding_with_the_whole_range_absent_round_trips_without_it() {
    let mut finding = spanned_finding();
    let object = finding.as_object_mut().unwrap();
    for key in ["span", "end_line", "end_column"] {
        object.remove(key);
    }
    let parsed: Finding = serde_json::from_value(finding.clone()).unwrap();
    assert!(parsed.extent.is_none());
    assert_eq!(serde_json::to_value(&parsed).unwrap(), finding);
}

#[test]
fn finding_with_the_whole_range_present_round_trips() {
    let finding = spanned_finding();
    let parsed: Finding = serde_json::from_value(finding.clone()).unwrap();
    assert!(parsed.extent.is_some());
    assert_eq!(serde_json::to_value(&parsed).unwrap(), finding);
}

#[test]
fn finding_rejects_duplicate_contexts() {
    let mut finding = spanned_finding();
    finding["contexts"] = json!(["prose", "prose"]);
    assert!(serde_json::from_value::<Finding>(finding).is_err());
}

#[test]
fn finding_rejects_a_zero_column() {
    let mut finding = spanned_finding();
    finding["column"] = json!(0);
    assert!(serde_json::from_value::<Finding>(finding).is_err());
}

#[test]
fn null_position_fixtures_do_not_decode_as_reports() {
    for name in ["finding-span-null.json", "finding-end-line-null.json"] {
        assert!(
            serde_json::from_value::<Report>(fixture("invalid", name)).is_err(),
            "{name} must not decode"
        );
    }
}

#[test]
fn contexts_construction_rejects_duplicates() {
    let prose = ContextTag::new("prose").unwrap();
    assert!(Contexts::new(vec![prose.clone(), prose.clone()]).is_err());
    let table = ContextTag::new("table-cell").unwrap();
    assert_eq!(Contexts::new(vec![prose, table]).unwrap().len(), 2);
}

#[test]
fn document_rejects_duplicate_contexts_seen() {
    let mut report = fixture("valid", "multi-document.json");
    patch_layer(&mut report);
    assert!(
        serde_json::from_value::<Report>(report.clone()).is_ok(),
        "the baseline must decode before duplicates are injected"
    );
    report["documents"][0]["contexts_seen"] = json!(["prose", "prose"]);
    let error = serde_json::from_value::<Report>(report).unwrap_err();
    assert!(
        error.to_string().contains("listed twice"),
        "unexpected error: {error}"
    );
}

#[test]
fn finding_rejects_every_optional_member_set_to_null() {
    for key in ["span", "end_line", "end_column", "fix", "provenance"] {
        let mut finding = spanned_finding();
        finding[key] = Value::Null;
        assert!(
            serde_json::from_value::<Finding>(finding).is_err(),
            "{key}: null must not read as absent"
        );
    }
}

#[test]
fn finding_rejects_the_whole_range_set_to_null() {
    let mut finding = spanned_finding();
    for key in ["span", "end_line", "end_column"] {
        finding[key] = Value::Null;
    }
    assert!(serde_json::from_value::<Finding>(finding).is_err());
}

#[test]
fn optional_members_reject_null_elsewhere() {
    assert!(
        serde_json::from_value::<Notice>(
            json!({"code": "a-b", "level": "info", "message": "m", "path": null})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<ConfigSource>(json!({"layer": "core", "line": null})).is_err()
    );
    assert!(
        serde_json::from_value::<RuleProvenance>(json!({"pack": "core", "version": null})).is_err()
    );
}

#[test]
fn category_score_budget_must_be_present_but_may_be_null() {
    let mut report = fixture("valid", "multi-document.json");
    patch_layer(&mut report);
    report["documents"][0]["categories"][0]
        .as_object_mut()
        .unwrap()
        .remove("budget");
    assert!(serde_json::from_value::<Report>(report).is_err());
}

#[test]
fn fix_construction_rejects_no_edits_and_overlap() {
    assert!(Fix::new(vec![]).is_err());
    assert!(Fix::new(vec![edit(0, 5), edit(3, 8)]).is_err());
    let touching = Fix::new(vec![edit(0, 5), edit(5, 8)]).unwrap();
    assert_eq!(touching.edits().len(), 2);
}

#[test]
fn fix_deserialize_rejects_no_edits_and_overlap() {
    assert!(serde_json::from_value::<Fix>(json!({"edits": []})).is_err());
    let overlapping = json!({"edits": [
        {"span": {"start": 0, "end": 5}, "text": "a"},
        {"span": {"start": 3, "end": 8}, "text": "b"}
    ]});
    assert!(serde_json::from_value::<Fix>(overlapping).is_err());
}
