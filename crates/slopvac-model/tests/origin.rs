//! `Finding.origin`: limits, range order, the origin/fix exclusion and the round trip.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

mod common;

use common::{corpus, load};
use rstest::rstest;
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

/// Each field's limit: `n` code points pass, `n + 1` fail. ASCII and astral characters count
/// once each (U+1F600 is 4 UTF-8 bytes and 2 UTF-16 units).
#[rstest]
#[case::label_ascii_at_limit(|n| OriginLabel::new("a".repeat(n)).is_ok(), 256, true)]
#[case::label_ascii_over_limit(|n| OriginLabel::new("a".repeat(n)).is_ok(), 257, false)]
#[case::label_multibyte_at_limit(|n| OriginLabel::new("é".repeat(n)).is_ok(), 256, true)]
#[case::label_multibyte_over_limit(|n| OriginLabel::new("é".repeat(n)).is_ok(), 257, false)]
#[case::label_astral_at_limit(|n| OriginLabel::new("\u{1F600}".repeat(n)).is_ok(), 256, true)]
#[case::label_astral_over_limit(|n| OriginLabel::new("\u{1F600}".repeat(n)).is_ok(), 257, false)]
#[case::locator_ascii_at_limit(|n| OriginLocator::new("a".repeat(n)).is_ok(), 3072, true)]
#[case::locator_ascii_over_limit(|n| OriginLocator::new("a".repeat(n)).is_ok(), 3073, false)]
#[case::locator_cjk_at_limit(|n| OriginLocator::new("中".repeat(n)).is_ok(), 3072, true)]
#[case::locator_cjk_over_limit(|n| OriginLocator::new("中".repeat(n)).is_ok(), 3073, false)]
#[case::locator_astral_at_limit(|n| OriginLocator::new("\u{1F600}".repeat(n)).is_ok(), 3072, true)]
#[case::locator_astral_over_limit(|n| OriginLocator::new("\u{1F600}".repeat(n)).is_ok(), 3073, false)]
#[case::segment_id_at_limit(|n| SegmentId::new("a".repeat(n)).is_ok(), 64, true)]
#[case::segment_id_over_limit(|n| SegmentId::new("a".repeat(n)).is_ok(), 65, false)]
fn field_limit(#[case] accepts: fn(usize) -> bool, #[case] count: usize, #[case] expected: bool) {
    assert_eq!(accepts(count), expected);
}

/// An extractor locator of 1024 characters, each a `%` (one of the characters the report
/// escapes to three), becomes `%25` x 1024 = 3072 code points in the report. The report
/// accepts it and keeps it byte for byte. There is no Rust extractor yet, so this drives the
/// public `Finding` decode and encode with the escaped form.
#[test]
fn longest_extractor_locator_fits_the_report_origin_once_escaped() {
    let escaped = "%25".repeat(1024);
    assert_eq!(escaped.chars().count(), 3072);
    let mut value = finding_with_origin();
    value["origin"]["locator"] = json!(escaped);
    let finding: Finding = serde_json::from_value(value.clone()).unwrap();
    let origin = finding.origin.as_ref().unwrap();
    assert_eq!(origin.locator().as_str(), escaped);
    assert_eq!(serde_json::to_value(&finding).unwrap(), value);
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
