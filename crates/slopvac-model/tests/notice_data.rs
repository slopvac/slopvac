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
use std::num::NonZeroUsize;

use slopvac_model::{
    AuthorSource, ItemLocation, ItemRole, Notice, NoticeData, NoticeError, NoticeLevel,
    RequestedSource, ReviewerSource,
};

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
    /// A notice for a refused document budget.
    #[code("context-budget-exceeded")]
    #[notice("context budget")]
    ContextBudget,
    /// A notice for a refused config budget.
    #[code("config-budget-exceeded")]
    #[notice("config budget")]
    ConfigBudget,
    /// A notice for a rule id no rule has.
    #[code("config-unknown-rule")]
    #[notice("unknown rule")]
    UnknownRule,
    /// A notice for a reviewer that is the author's model.
    #[code("review-model-same")]
    #[notice("same model")]
    ModelSame,
    /// A notice for a reported model that is not the requested one.
    #[code("review-model-reported-mismatch")]
    #[notice("model mismatch")]
    ModelMismatch,
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

fn at(line: usize) -> NonZeroUsize {
    NonZeroUsize::new(line).unwrap()
}

fn items() -> Vec<ItemLocation> {
    vec![
        ItemLocation::new(ItemRole::OriginalDocument, at(1)),
        ItemLocation::new(ItemRole::RevisedDocument, at(2)),
    ]
}

/// The `data` object that a notice built from `data` writes.
fn data_json(message: &Message, data: NoticeData) -> String {
    let notice = Notice::try_from_catalog(NoticeLevel::Warning, message)
        .unwrap()
        .with(data)
        .unwrap();
    serde_json::to_string(&notice.data).unwrap()
}

#[rstest]
#[case::context(
    &Message::ContextBudget,
    NoticeData::context_budget_exceeded("tags", 70_000, "docs/a.md"),
    r#"{"budget":"tags","limit":70000,"path":"docs/a.md"}"#
)]
#[case::config(
    &Message::ConfigBudget,
    NoticeData::config_budget_exceeded("extends-depth", 32, "<external>/b.toml"),
    r#"{"budget":"extends-depth","limit":32,"path":"<external>/b.toml"}"#
)]
#[case::unknown_rule_with_position(
    &Message::UnknownRule,
    NoticeData::config_unknown_rule("core:a.b", Some(at(4)), Some(at(2))),
    r#"{"rule":"core:a.b","line":4,"column":2}"#
)]
#[case::unknown_rule_without_position_omits_both_keys(
    &Message::UnknownRule,
    NoticeData::config_unknown_rule("a.b", None, None),
    r#"{"rule":"a.b"}"#
)]
#[case::unknown_rule_with_a_line_only(
    &Message::UnknownRule,
    NoticeData::config_unknown_rule("a.b", Some(at(7)), None),
    r#"{"rule":"a.b","line":7}"#
)]
#[case::same_model_declared_by_the_provider_has_no_items(
    &Message::ModelSame,
    NoticeData::review_model_same("m", AuthorSource::Flag, "m", ReviewerSource::Provider, vec![]).unwrap(),
    r#"{"author":"m","author_source":"flag","reviewer":"m","reviewer_source":"provider"}"#
)]
#[case::same_model_reported_lists_its_items(
    &Message::ModelSame,
    NoticeData::review_model_same("m", AuthorSource::Unset, "n", ReviewerSource::Reported, items()).unwrap(),
    r#"{"author":"m","author_source":"none","reviewer":"n","reviewer_source":"reported","items":[{"role":"original_document","line":1},{"role":"revised_document","line":2}]}"#
)]
#[case::mismatch(
    &Message::ModelMismatch,
    NoticeData::review_model_reported_mismatch("a", RequestedSource::Config, "b", items()).unwrap(),
    r#"{"requested":"a","requested_source":"config","reported":"b","items":[{"role":"original_document","line":1},{"role":"revised_document","line":2}]}"#
)]
fn the_new_codes_write_the_documented_keys_and_no_null(
    #[case] message: &Message,
    #[case] data: NoticeData,
    #[case] expected: &str,
) {
    assert_eq!(data_json(message, data), expected);
}

#[rstest]
#[case::reported_without_items(ReviewerSource::Reported, vec![])]
#[case::flag_with_items(ReviewerSource::Flag, items())]
#[case::config_with_items(ReviewerSource::Config, items())]
#[case::provider_with_items(ReviewerSource::Provider, items())]
fn same_model_data_needs_items_exactly_when_the_reviewer_is_reported(
    #[case] source: ReviewerSource,
    #[case] listed: Vec<ItemLocation>,
) {
    let refused = NoticeData::review_model_same("m", AuthorSource::Flag, "m", source, listed).err();
    assert_eq!(
        refused,
        Some(NoticeError::DataShape("review-model-same".to_owned()))
    );
}

#[test]
fn mismatch_data_without_items_is_refused() {
    let refused =
        NoticeData::review_model_reported_mismatch("a", RequestedSource::Flag, "b", vec![]).err();
    assert_eq!(
        refused,
        Some(NoticeError::DataShape(
            "review-model-reported-mismatch".to_owned()
        ))
    );
}

#[rstest]
#[case::path_budget_on_unknown_rule(
    &Message::UnknownRule,
    NoticeData::context_budget_exceeded("tags", 1, "a.md")
)]
#[case::context_data_on_a_config_budget(
    &Message::ConfigBudget,
    NoticeData::context_budget_exceeded("tags", 1, "a.md")
)]
#[case::unknown_rule_on_a_model_code(
    &Message::ModelSame,
    NoticeData::config_unknown_rule("a.b", None, None)
)]
#[case::budget_on_a_path_budget_code(
    &Message::ContextBudget,
    NoticeData::extract_budget("fragments", 1)
)]
fn data_of_another_code_is_refused_by_the_setter_for_the_new_codes(
    #[case] message: &Message,
    #[case] data: NoticeData,
) {
    let notice = Notice::try_from_catalog(NoticeLevel::Warning, message).unwrap();
    assert!(matches!(
        notice.with(data).err(),
        Some(NoticeError::DataShape(_))
    ));
}

#[test]
fn an_item_role_is_written_in_snake_case() {
    let roles = [
        ItemRole::Changed,
        ItemRole::Added,
        ItemRole::Dropped,
        ItemRole::OriginalDocument,
        ItemRole::RevisedDocument,
    ];
    let written: Vec<String> = roles
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect();
    assert_eq!(
        written,
        [
            r#""changed""#,
            r#""added""#,
            r#""dropped""#,
            r#""original_document""#,
            r#""revised_document""#
        ]
    );
}
