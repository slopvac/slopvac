//! Snapshots of rendered diagnostics, the text users read.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use slopvac_config::tree::parse_toml;
use slopvac_config::{SourceFile, render, validate_config};

fn rendered(text: &str) -> String {
    let root = parse_toml(text).unwrap();
    let diags = validate_config(&root, text);
    render(
        SourceFile {
            name: "slopvac.toml",
            text,
        },
        &diags,
    )
}

#[test]
fn unknown_key_suggests_the_nearest_key() {
    insta::assert_snapshot!(rendered(
        "schema_version = 3\n\n[thresholds]\nmax_eror = 0\n"
    ));
}

#[test]
fn bad_level_suggests_the_nearest_level() {
    insta::assert_snapshot!(rendered(
        "schema_version = 3\n\n[rules]\n\"core:no-dash\" = \"of\"\n"
    ));
}

#[test]
fn rule_table_form_explains_the_bare_level() {
    insta::assert_snapshot!(rendered(
        "schema_version = 3\n\n[rules.\"core:no-dash\"]\nseverity = \"off\"\n"
    ));
}

#[test]
fn whole_run_table_in_an_override_is_named() {
    insta::assert_snapshot!(rendered(
        "schema_version = 3\n\n[[overrides]]\nfiles = [\"docs/**\"]\n\n[overrides.guidance]\nlevel = \"full\"\n"
    ));
}

#[test]
fn missing_required_key_points_at_the_table_header() {
    insta::assert_snapshot!(rendered(
        "schema_version = 3\n\n[[overrides]]\nlocale = \"en-GB\"\n"
    ));
}

#[test]
fn out_of_range_value_states_the_bounds() {
    insta::assert_snapshot!(rendered(
        "schema_version = 3\n\n[thresholds]\nmin_score = 120\n"
    ));
}

#[test]
fn multiple_problems_render_in_document_order() {
    insta::assert_snapshot!(rendered(
        "schema_version = 3\nlocale = \"fr\"\nmodee = \"prose\"\n"
    ));
}

fn rendered_with(text: &str, name: &str) -> String {
    let root = parse_toml(text).unwrap();
    let diags = validate_config(&root, text);
    render(SourceFile { name, text }, &diags)
}

#[test]
fn rendered_positions_follow_the_position_contract() {
    // The same mistake on line 3, column 1, written with each line terminator.
    for (label, text) in [
        ("lf", "schema_version = 3\n\nmodee = \"prose\"\n"),
        ("crlf", "schema_version = 3\r\n\r\nmodee = \"prose\"\r\n"),
        ("bom", "\u{feff}schema_version = 3\n\nmodee = \"prose\"\n"),
    ] {
        let out = rendered_with(text, "slopvac.toml");
        assert!(out.contains("--> slopvac.toml:3:1"), "{label}: {out}");
        assert!(out.contains("3 | modee = \"prose\""), "{label}: {out}");
    }
}

#[test]
fn a_mistake_on_the_first_line_after_a_byte_order_mark_is_column_one() {
    let out = rendered_with(
        "\u{feff}modee = \"prose\"\nschema_version = 3\n",
        "slopvac.toml",
    );
    assert!(out.contains("--> slopvac.toml:1:1"), "{out}");
}

#[test]
fn a_lone_cr_ends_a_rendered_line() {
    // TOML refuses a lone CR, but YAML front matter and directives allow one.
    for (label, text) in [("cr", "ab\rcd\ref"), ("bom and cr", "\u{feff}ab\rcd\ref")] {
        let at = text.find("ef").unwrap();
        let d = slopvac_config::Diagnostic {
            code: slopvac_config::Code::UnknownKey,
            message: "unknown key".into(),
            label: "here".into(),
            span: at..at + 2,
            help: None,
            path: vec![],
            data: None,
        };
        let out = render(SourceFile { name: "f.md", text }, &[d]);
        assert!(out.contains("--> f.md:3:1"), "{label}: {out}");
        assert!(out.contains("3 | ef"), "{label}: {out}");
    }
}
