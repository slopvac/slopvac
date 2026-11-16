//! Schema rules that the shared corpus does not cover, through the public loader.

use slopvac_styles::{Style, StyleError};

/// `(what, body after schema_version, code, JSON pointer)`
const REJECTED: &[(&str, &str, &str, &str)] = &[
    (
        "empty description",
        "description = \"\"",
        "config-invalid-value",
        "/description",
    ),
    (
        "empty allow item",
        "[terms]\nallow = [\"\"]",
        "config-invalid-value",
        "/terms/allow/0",
    ),
    (
        "duplicate extends",
        "extends = [\"a\", \"a\"]",
        "config-invalid-value",
        "/extends",
    ),
    (
        "duplicate conflicts",
        "conflicts = [\"a\", \"a\"]",
        "config-invalid-value",
        "/conflicts",
    ),
    (
        "malformed rule key",
        "[rules]\n\"Bad Key\" = \"off\"",
        "config-invalid-value",
        "/rules/Bad Key",
    ),
    (
        "malformed category key",
        "[categories]\n\"Bad Key\" = \"off\"",
        "config-invalid-value",
        "/categories/Bad Key",
    ),
];

#[test]
fn the_loader_rejects_each_case_at_its_pointer() {
    for (what, body, code, pointer) in REJECTED {
        let text = format!("schema_version = 3\n{body}\n");
        let Err(StyleError::Invalid { problems, .. }) = Style::parse("sample", &text) else {
            panic!("{what}: expected an invalid-style error");
        };
        assert!(
            problems
                .iter()
                .any(|(c, p)| c == code && p.starts_with(pointer)),
            "{what}: expected {code} at `{pointer}`, got {problems:?}"
        );
    }
}

#[test]
fn malformed_toml_reports_the_position_of_the_parser() {
    let Err(StyleError::Parse { style, message }) =
        Style::parse("broken", "schema_version = 3\n[rules\n")
    else {
        panic!("expected a parse error");
    };
    assert_eq!(style, "broken");
    assert!(
        message.contains("preset:broken:2:"),
        "no file:line:col in {message}"
    );
}
