//! Typed notice data: the JSON a notice writes stays the shape the contract documents.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use slopvac_messages::Catalog;
use slopvac_model::{Notice, NoticeData, NoticeLevel};

#[derive(Debug, Catalog)]
enum Message {
    /// A notice that carries a rule list.
    #[code("rule-kind-not-implemented")]
    #[notice("rules did not run")]
    Rules,
    /// A notice for a diagram type no extractor reads.
    #[code("extract-unsupported")]
    #[notice("unsupported diagram")]
    Unsupported,
    /// A notice for an extraction cap.
    #[code("extract-budget")]
    #[notice("budget exhausted")]
    Budget,
}

fn notice(message: &Message, level: NoticeLevel, data: NoticeData) -> String {
    let notice = Notice::try_from_catalog(level, message).unwrap().with(data);
    serde_json::to_string(&notice).unwrap()
}

#[test]
fn rules_data_writes_a_list_of_rule_ids() {
    let json = notice(
        &Message::Rules,
        NoticeLevel::Warning,
        NoticeData::rules(["core:a.b", "core:c.d"]),
    );
    assert_eq!(
        json,
        r#"{"code":"rule-kind-not-implemented","level":"warning","message":"rules did not run","data":{"rules":["core:a.b","core:c.d"]}}"#
    );
}

#[test]
fn unsupported_data_writes_document_and_kind() {
    let json = notice(
        &Message::Unsupported,
        NoticeLevel::Info,
        NoticeData::unsupported("docs/a.md", "mermaid"),
    );
    assert_eq!(
        json,
        r#"{"code":"extract-unsupported","level":"info","message":"unsupported diagram","data":{"document":"docs/a.md","kind":"mermaid"}}"#
    );
}

#[test]
fn budget_data_writes_the_cap_name_and_its_numeric_limit() {
    let json = notice(
        &Message::Budget,
        NoticeLevel::Warning,
        NoticeData::budget("fragments", 1000),
    );
    assert_eq!(
        json,
        r#"{"code":"extract-budget","level":"warning","message":"budget exhausted","data":{"budget":"fragments","limit":1000}}"#
    );
}

#[test]
fn typed_data_is_readable_through_the_untyped_map() {
    let notice = Notice::try_from_catalog(NoticeLevel::Warning, &Message::Budget)
        .unwrap()
        .with(NoticeData::budget("xml-nodes", 7));
    assert_eq!(notice.data["budget"], "xml-nodes");
    assert_eq!(notice.data["limit"], 7);
}
