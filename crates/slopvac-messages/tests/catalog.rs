//! Behaviour of `#[derive(Catalog)]` through the public API.

mod support;

use std::error::Error as _;

use slopvac_messages::{Arg, Catalog, Kind, Param, ParamKind, Value, render};
use support::{Help, ReadError, RunError};

fn unreadable() -> ReadError {
    ReadError::Unreadable {
        path: "docs/a.md".into(),
        reason: "permission denied".into(),
        cause: std::io::Error::other("denied"),
    }
}

#[test]
fn renders_text_and_path_parameters_into_the_template() {
    assert_eq!(
        unreadable().to_string(),
        "cannot read docs/a.md: permission denied"
    );
}

#[test]
fn display_and_render_agree() {
    let error = unreadable();
    assert_eq!(render(&error), error.to_string());
}

#[test]
fn renders_counts_as_decimal_numbers() {
    let error = ReadError::TooMany {
        found: 12,
        limit: 10,
    };
    assert_eq!(error.to_string(), "found 12 files, the limit is 10");
}

#[test]
fn doubled_braces_are_literal() {
    let error = ReadError::Braces { key: "rule".into() };
    assert_eq!(error.to_string(), "write {key} to name rule");
}

#[test]
fn line_breaks_in_a_template_are_kept() {
    let error = ReadError::TwoLines { key: "k".into() };
    assert_eq!(error.to_string(), "first line\nsecond k\n\n.dotted line");
}

#[test]
fn entry_carries_code_kind_key_summary_and_params() {
    let entry = unreadable().entry();
    assert_eq!(entry.code, "file-unreadable");
    assert_eq!(entry.kind, Kind::Error);
    assert_eq!(entry.key, "ReadError::Unreadable");
    assert_eq!(entry.summary, "The file could not be read.");
    assert_eq!(entry.template, "cannot read {path}: {reason}");
    assert_eq!(
        entry.params,
        [
            Param::new("path", ParamKind::Path),
            Param::new("reason", ParamKind::Text)
        ]
    );
}

#[test]
fn count_parameters_are_marked_as_counts() {
    let params = ReadError::TooMany { found: 1, limit: 2 }.entry().params;
    assert!(params.iter().all(|p| p.kind == ParamKind::Count));
}

#[test]
fn args_hold_plain_values_in_field_order() {
    assert_eq!(
        unreadable().args(),
        [
            Arg::new("path", Value::Path("docs/a.md".into())),
            Arg::new("reason", Value::Text("permission denied".into())),
        ]
    );
}

#[test]
fn source_fields_are_not_parameters_and_stay_the_error_source() {
    let error = unreadable();
    assert_eq!(error.args().len(), 2);
    assert_eq!(error.source().expect("a source").to_string(), "denied");
}

#[test]
fn entries_list_every_variant_in_order() {
    let codes: Vec<_> = ReadError::entries().iter().map(|e| e.code).collect();
    assert_eq!(
        codes,
        [
            "file-unreadable",
            "too-many-files",
            "brace-example",
            "two-lines"
        ]
    );
}

#[test]
fn a_delegating_variant_renders_as_the_wrapped_message() {
    let wrapped = RunError::from(ReadError::TooMany { found: 3, limit: 1 });
    assert_eq!(wrapped.code(), "too-many-files");
    assert_eq!(wrapped.to_string(), "found 3 files, the limit is 1");
}

#[test]
fn a_delegating_variant_adds_no_entry_of_its_own() {
    let codes: Vec<_> = RunError::entries().iter().map(|e| e.code).collect();
    assert_eq!(codes, ["run-cancelled"]);
}

#[test]
fn a_named_delegate_field_works_too() {
    let wrapped = RunError::Named {
        inner: ReadError::Braces { key: "x".into() },
    };
    assert_eq!(wrapped.code(), "brace-example");
}

#[test]
fn as_static_gives_the_fixed_text_with_braces_unescaped() {
    assert_eq!(Help::Check.as_static(), "Fail when a finding remains");
    assert_eq!(Help::Braces {}.as_static(), "Use {name} placeholders");
}

#[test]
fn kind_follows_the_template_attribute() {
    assert_eq!(Help::Check.entry().kind, Kind::Text);
    assert_eq!(Help::Braces {}.entry().kind, Kind::Notice);
}
