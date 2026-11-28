//! The reader and the published schema judge the `data` of the review, path-budget and
//! unknown-rule notice codes the same way. Each case runs against both.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

mod common;

use common::{corpus, load, published_lint_report};
use rstest::rstest;
use serde_json::{Value, json};
use slopvac_model::Notice;

/// Whether the Rust reader takes the notice.
fn read_accepts(code: &str, data: &Value) -> bool {
    serde_json::from_value::<Notice>(json!({
        "code": code, "level": "warning", "message": "m", "data": data
    }))
    .is_ok()
}

/// Whether the published schema takes a report that holds the notice.
fn schema_accepts(code: &str, data: &Value) -> bool {
    let mut report = load(&corpus().join("lint-report/valid/empty-run.json"));
    report["notices"] = json!([{"code": code, "level": "warning", "message": "m", "data": data}]);
    published_lint_report().is_valid(&report)
}

fn with(mut data: Value, extra: &Value) -> Value {
    for (key, value) in extra.as_object().unwrap() {
        data[key] = value.clone();
    }
    data
}

fn same(extra: &Value) -> Value {
    with(
        json!({
            "author": "a-model", "author_source": "flag",
            "reviewer": "b-model", "reviewer_source": "config",
        }),
        extra,
    )
}

fn mismatch(extra: &Value) -> Value {
    with(
        json!({
            "requested": "a-model", "requested_source": "flag", "reported": "b-model",
            "items": [{"role": "changed", "line": 3}],
        }),
        extra,
    )
}

fn item(role: &str, line: &Value) -> Value {
    json!([{"role": role, "line": line}])
}

#[rstest]
#[case::context_budget("context-budget-exceeded", json!({"budget": "tags", "limit": 70_000, "path": "docs/a.md"}))]
#[case::config_budget("config-budget-exceeded", json!({"budget": "extends-depth", "limit": 32, "path": "slopvac.toml"}))]
#[case::path_budget_extra_key("context-budget-exceeded", json!({"budget": "tags", "limit": 1, "path": "<stdin>", "x": 1}))]
#[case::unknown_rule_bare("config-unknown-rule", json!({"rule": "core:a.b"}))]
#[case::unknown_rule_with_position("config-unknown-rule", json!({"rule": "a.b", "line": 4, "column": 1}))]
#[case::same_without_items("review-model-same", same(&json!({})))]
#[case::same_provider_without_items("review-model-same", same(&json!({"reviewer_source": "provider"})))]
#[case::same_none_author("review-model-same", same(&json!({"author_source": "none"})))]
#[case::same_reported_with_items("review-model-same", same(&json!({
    "reviewer_source": "reported",
    "items": [{"role": "original_document", "line": 1}, {"role": "revised_document", "line": 2}],
})))]
#[case::mismatch("review-model-reported-mismatch", mismatch(&json!({})))]
#[case::mismatch_config("review-model-reported-mismatch", mismatch(&json!({"requested_source": "config"})))]
fn a_shape_that_matches_its_code_is_read_and_valid(#[case] code: &str, #[case] data: Value) {
    assert!(read_accepts(code, &data), "reader refused {data}");
    assert!(schema_accepts(code, &data), "schema refused {data}");
}

#[rstest]
#[case::context_path_missing("context-budget-exceeded", json!({"budget": "tags", "limit": 1}))]
#[case::config_path_missing("config-budget-exceeded", json!({"budget": "extends-depth", "limit": 1}))]
#[case::path_wrong_type("context-budget-exceeded", json!({"budget": "tags", "limit": 1, "path": 5}))]
#[case::path_budget_limit_negative("config-budget-exceeded", json!({"budget": "x", "limit": -1, "path": "a"}))]
#[case::path_budget_name_wrong_type("context-budget-exceeded", json!({"budget": 1, "limit": 1, "path": "a"}))]
#[case::unknown_rule_missing("config-unknown-rule", json!({"line": 1}))]
#[case::unknown_rule_wrong_type("config-unknown-rule", json!({"rule": 5}))]
#[case::line_zero("config-unknown-rule", json!({"rule": "a.b", "line": 0}))]
#[case::column_zero("config-unknown-rule", json!({"rule": "a.b", "column": 0}))]
#[case::line_null("config-unknown-rule", json!({"rule": "a.b", "line": null}))]
#[case::column_null("config-unknown-rule", json!({"rule": "a.b", "column": null}))]
#[case::line_fraction("config-unknown-rule", json!({"rule": "a.b", "line": 1.5}))]
#[case::same_author_missing("review-model-same", json!({"author_source": "flag", "reviewer": "b", "reviewer_source": "flag"}))]
#[case::same_reviewer_wrong_type("review-model-same", same(&json!({"reviewer": 5})))]
#[case::same_author_source_provider("review-model-same", same(&json!({"author_source": "provider"})))]
#[case::same_author_source_reported("review-model-same", same(&json!({"author_source": "reported"})))]
#[case::same_reviewer_source_none("review-model-same", same(&json!({"reviewer_source": "none"})))]
#[case::same_reported_without_items("review-model-same", same(&json!({"reviewer_source": "reported"})))]
#[case::same_reported_with_empty_items("review-model-same", same(&json!({"reviewer_source": "reported", "items": []})))]
#[case::same_reported_with_null_items("review-model-same", same(&json!({"reviewer_source": "reported", "items": null})))]
#[case::same_flag_with_items("review-model-same", same(&json!({"reviewer_source": "flag", "items": item("changed", &json!(1))})))]
#[case::same_provider_with_items("review-model-same", same(&json!({"reviewer_source": "provider", "items": item("changed", &json!(1))})))]
#[case::same_item_role_unknown("review-model-same", same(&json!({"reviewer_source": "reported", "items": item("other", &json!(1))})))]
#[case::same_item_line_zero("review-model-same", same(&json!({"reviewer_source": "reported", "items": item("added", &json!(0))})))]
#[case::same_item_line_missing("review-model-same", same(&json!({"reviewer_source": "reported", "items": [{"role": "added"}]})))]
#[case::mismatch_requested_missing("review-model-reported-mismatch", json!({"requested_source": "flag", "reported": "b", "items": item("added", &json!(1))}))]
#[case::mismatch_items_missing("review-model-reported-mismatch", json!({"requested": "a", "requested_source": "flag", "reported": "b"}))]
#[case::mismatch_items_empty("review-model-reported-mismatch", mismatch(&json!({"items": []})))]
#[case::mismatch_items_null("review-model-reported-mismatch", mismatch(&json!({"items": null})))]
#[case::mismatch_source_provider("review-model-reported-mismatch", mismatch(&json!({"requested_source": "provider"})))]
#[case::mismatch_source_none("review-model-reported-mismatch", mismatch(&json!({"requested_source": "none"})))]
#[case::mismatch_reported_wrong_type("review-model-reported-mismatch", mismatch(&json!({"reported": []})))]
#[case::mismatch_item_role_unknown("review-model-reported-mismatch", mismatch(&json!({"items": item("other", &json!(1))})))]
#[case::mismatch_item_line_zero("review-model-reported-mismatch", mismatch(&json!({"items": item("changed", &json!(0))})))]
fn a_shape_off_its_code_is_refused_by_the_reader_and_the_schema(
    #[case] code: &str,
    #[case] data: Value,
) {
    assert!(!read_accepts(code, &data), "reader took {data}");
    assert!(!schema_accepts(code, &data), "schema took {data}");
}
