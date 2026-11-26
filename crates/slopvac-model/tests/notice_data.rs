//! Typed notice data: the JSON a notice writes stays the shape the contract documents.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use rstest::rstest;
use serde_json::json;
use slopvac_messages::Catalog;
use slopvac_model::{Notice, NoticeData, NoticeError, NoticeLevel};

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
    let notice = Notice::try_from_catalog(level, message)
        .unwrap()
        .with(data)
        .unwrap();
    serde_json::to_string(&notice).unwrap()
}

#[test]
fn rules_data_writes_a_list_of_rule_ids() {
    let json = notice(
        &Message::Rules,
        NoticeLevel::Warning,
        NoticeData::rule_kind_not_implemented(["core:a.b", "core:c.d"]),
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
        NoticeData::extract_unsupported("docs/a.md", "mermaid"),
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
        NoticeData::extract_budget("fragments", 1000),
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
        .with(NoticeData::extract_budget("xml-nodes", 7))
        .unwrap();
    assert_eq!(notice.data["budget"], "xml-nodes");
    assert_eq!(notice.data["limit"], 7);
}

#[rstest]
#[case::rules_on_a_budget_code(Message::Budget, NoticeData::rule_kind_not_implemented(["core:a.b"]))]
#[case::budget_on_a_rules_code(Message::Rules, NoticeData::extract_budget("fragments", 1))]
#[case::unsupported_on_a_budget_code(
    Message::Budget,
    NoticeData::extract_unsupported("a.md", "mermaid")
)]
fn data_of_another_shape_is_refused_by_the_setter(
    #[case] message: Message,
    #[case] data: NoticeData,
) {
    let notice = Notice::try_from_catalog(NoticeLevel::Warning, &message).unwrap();
    let refused = notice.with(data).unwrap_err();
    assert!(matches!(refused, NoticeError::DataShape(_)));
}

#[test]
fn a_notice_built_from_data_takes_its_code_from_the_data_and_its_text_from_the_message() {
    let message = slopvac_model::NoticeMessage::try_from_catalog(&Message::Rules).unwrap();
    let notice = Notice::from_catalog_data(
        NoticeLevel::Warning,
        message,
        NoticeData::extract_budget("fragments", 1000),
    );
    assert_eq!(
        serde_json::to_string(&notice).unwrap(),
        r#"{"code":"extract-budget","level":"warning","message":"rules did not run","data":{"budget":"fragments","limit":1000}}"#
    );
}

#[rstest]
#[case::kind(NoticeData::rule_kind_not_implemented(["a:b"]), "rule-kind-not-implemented")]
#[case::style(NoticeData::rule_style_not_implemented(["a:b"]), "rule-style-not-implemented")]
#[case::comment(NoticeData::comment_rules_not_run(["a:b"]), "comment-rules-not-run")]
#[case::named(NoticeData::comment_rule_named_not_run(["a:b"]), "comment-rule-named-not-run")]
#[case::cases(NoticeData::cases_rule_unchecked(["a:b"]), "cases-rule-unchecked")]
#[case::unsupported(
    NoticeData::extract_unsupported("a.md", "mermaid"),
    "extract-unsupported"
)]
#[case::budget(NoticeData::extract_budget("fragments", 1), "extract-budget")]
fn each_constructor_serves_one_code_and_builds_a_notice_that_reads_back(
    #[case] data: NoticeData,
    #[case] code: &str,
) {
    assert_eq!(data.code(), code);
    let message = slopvac_model::NoticeMessage::try_from_catalog(&Message::Rules).unwrap();
    let notice = Notice::from_catalog_data(NoticeLevel::Warning, message, data);
    let json = serde_json::to_string(&notice).unwrap();
    let read: Notice = serde_json::from_str(&json).unwrap();
    assert_eq!(read, notice);
    assert_eq!(read.code.as_str(), code);
}

fn rules_data(code: &str) -> NoticeData {
    match code {
        "rule-kind-not-implemented" => NoticeData::rule_kind_not_implemented(["a:b"]),
        "rule-style-not-implemented" => NoticeData::rule_style_not_implemented(["a:b"]),
        "comment-rules-not-run" => NoticeData::comment_rules_not_run(["a:b"]),
        "comment-rule-named-not-run" => NoticeData::comment_rule_named_not_run(["a:b"]),
        "cases-rule-unchecked" => NoticeData::cases_rule_unchecked(["a:b"]),
        other => panic!("not a rules code: {other}"),
    }
}

/// Five codes carry the same `rules` shape. Data built for one still belongs to that code only.
#[rstest]
fn data_of_another_code_with_the_same_shape_is_refused_by_the_setter(
    #[values(
        "rule-kind-not-implemented",
        "rule-style-not-implemented",
        "comment-rules-not-run",
        "comment-rule-named-not-run",
        "cases-rule-unchecked"
    )]
    notice_code: &str,
    #[values(
        "rule-kind-not-implemented",
        "rule-style-not-implemented",
        "comment-rules-not-run",
        "comment-rule-named-not-run",
        "cases-rule-unchecked"
    )]
    data_code: &str,
) {
    let notice: Notice =
        serde_json::from_value(json!({"code": notice_code, "level": "warning", "message": "m"}))
            .unwrap();
    let attached = notice.with(rules_data(data_code));
    if notice_code == data_code {
        assert_eq!(attached.unwrap().code.as_str(), notice_code);
    } else {
        assert_eq!(
            attached.unwrap_err(),
            NoticeError::DataShape(notice_code.to_owned())
        );
    }
}
