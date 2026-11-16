//! `Finding.origin`: limits, range order, the origin/fix exclusion and the round trip.

#![allow(missing_docs)]

mod common;

use common::{corpus, load};
use serde_json::{Value, json};
use slopvac_model::{
    Edit, Finding, FindingError, Fix, Origin, OriginLabel, OriginLocator, SegmentId, SegmentRange,
    Span,
};

fn origin_json() -> Value {
    json!({
        "locator": "xl/worksheets/sheet1.xml#B4%25",
        "label": "Sheet1!B4",
        "segment": "s0004",
        "range": {"start": 6, "end": 11}
    })
}

fn finding_with_origin() -> Value {
    load(&corpus().join("lint-report/valid/finding-origin.json"))["documents"][0]["findings"][1]
        .clone()
}

#[test]
fn origin_round_trips_exactly() {
    let value = origin_json();
    let origin: Origin = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(origin.locator().as_str(), "xl/worksheets/sheet1.xml#B4%25");
    assert_eq!(origin.label().as_str(), "Sheet1!B4");
    assert_eq!(origin.segment().as_str(), "s0004");
    assert_eq!((origin.range().start(), origin.range().end()), (6, 11));
    assert_eq!(serde_json::to_value(&origin).unwrap(), value);
}

#[test]
fn finding_keeps_its_origin_through_a_round_trip() {
    let value = finding_with_origin();
    assert_eq!(value["origin"]["segment"], "s0004");
    let finding: Finding = serde_json::from_value(value.clone()).unwrap();
    assert!(finding.origin.is_some());
    assert_eq!(serde_json::to_value(&finding).unwrap(), value);
}

#[test]
fn label_accepts_256_code_points_and_rejects_257() {
    assert!(OriginLabel::new("é".repeat(256)).is_ok());
    assert!(OriginLabel::new("é".repeat(257)).is_err());
}

#[test]
fn segment_id_accepts_64_characters_and_rejects_65() {
    assert!(SegmentId::new("a".repeat(64)).is_ok());
    assert!(SegmentId::new("a".repeat(65)).is_err());
}

#[test]
fn locator_accepts_512_code_points_and_rejects_513() {
    assert!(OriginLocator::new("中".repeat(512)).is_ok());
    assert!(OriginLocator::new("中".repeat(513)).is_err());
}

#[test]
fn reversed_range_is_rejected_by_constructor_and_decode() {
    assert!(SegmentRange::new(5, 4).is_err());
    assert!(SegmentRange::new(4, 4).is_ok());
    let mut value = origin_json();
    value["range"] = json!({"start": 11, "end": 6});
    assert!(serde_json::from_value::<Origin>(value).is_err());
}

#[test]
fn origin_with_fix_is_rejected_at_decode_and_at_serialize() {
    let mut value = finding_with_origin();
    value["fix"] = json!({"edits": [{"span": {"start": 0, "end": 1}, "text": "x"}]});
    let err = serde_json::from_value::<Finding>(value.clone()).unwrap_err();
    assert!(
        err.to_string().contains("origin cannot carry a fix"),
        "{err}"
    );

    let mut finding: Finding = serde_json::from_value(finding_with_origin()).unwrap();
    finding.fix = Some(
        Fix::new(vec![Edit {
            span: Span::new(0, 1).unwrap(),
            text: "x".into(),
        }])
        .unwrap(),
    );
    assert_eq!(finding.validate(), Err(FindingError::OriginWithFix));
    assert!(serde_json::to_value(&finding).is_err());
}
