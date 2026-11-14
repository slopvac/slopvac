//! Regressions found in review: each pins one behaviour of the public validators.

use slopvac_config::tree::{parse_toml, parse_yaml};
use slopvac_config::{Code, Diagnostic, validate_config, validate_frontmatter};

fn config(text: &str) -> Vec<Diagnostic> {
    validate_config(&parse_toml(text).unwrap(), text)
}

fn front_matter(text: &str) -> Vec<Diagnostic> {
    validate_frontmatter(&parse_yaml(text).unwrap(), text)
}

fn only(diags: &[Diagnostic]) -> &Diagnostic {
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:#?}");
    &diags[0]
}

#[test]
fn a_repeated_allow_entry_is_rejected_at_the_repeat() {
    let text = "schema_version = 3\n\n[frontmatter]\nallow = [\"genre\", \"genre\"]\n";
    let diags = config(text);
    let d = only(&diags);
    assert_eq!(d.code, Code::InvalidValue);
    assert_eq!(d.pointer(), "/frontmatter/allow/1");
    assert_eq!(d.location(text), (4, 19));
}

#[test]
fn distinct_allow_entries_are_accepted() {
    let text = "schema_version = 3\n\n[frontmatter]\nallow = [\"genre\", \"locale\"]\n";
    assert_eq!(config(text), []);
}

#[test]
fn malformed_yaml_is_an_error_that_points_into_the_text() {
    let text = "slopvac:\n  v: 3\n  genre: [a, b\n";
    let err = parse_yaml(text).unwrap_err();
    let span = err.span.expect("the parser reports where it stopped");
    assert!(span.start <= text.len());
    let line = text[..span.start].matches('\n').count() + 1;
    assert!(
        line >= 3,
        "error at line {line}, expected at or after the broken flow sequence"
    );
    assert_ne!(err.message, "");
}

#[test]
fn malformed_yaml_with_multibyte_text_keeps_a_valid_byte_offset() {
    let text = "# é — note\nslopvac: [\n";
    let span = parse_yaml(text).unwrap_err().span.unwrap();
    assert!(text.is_char_boundary(span.start));
}

#[test]
fn a_front_matter_root_that_is_not_a_mapping_is_rejected() {
    for text in ["- a\n- b\n", "just text\n", "", "42\n"] {
        let diags = front_matter(text);
        let d = only(&diags);
        assert_eq!(d.code, Code::InvalidType, "{text:?}");
        assert_eq!(d.pointer(), "", "{text:?}");
        assert!(d.message.contains("expected a mapping"), "{text:?}");
    }
}

#[test]
fn a_mapping_without_a_slopvac_key_has_nothing_to_check() {
    assert_eq!(front_matter("title: x\n"), []);
}

#[test]
fn non_finite_numbers_are_rejected_in_toml() {
    for value in ["nan", "inf", "-inf"] {
        let text =
            format!("schema_version = 3\n\n[thresholds]\nmax_total_per_100_words = {value}\n");
        let diags = config(&text);
        let d = only(&diags);
        assert_eq!(d.code, Code::InvalidValue, "{value}");
        assert_eq!(
            d.pointer(),
            "/thresholds/max_total_per_100_words",
            "{value}"
        );
    }
}

#[test]
fn non_finite_numbers_are_rejected_in_yaml() {
    for value in [".nan", ".inf", "-.inf"] {
        let text = format!("slopvac:\n  v: 3\n  thresholds:\n    min_score: {value}\n");
        let diags = front_matter(&text);
        let d = only(&diags);
        assert_eq!(d.code, Code::InvalidValue, "{value}");
        assert_eq!(d.pointer(), "/slopvac/thresholds/min_score", "{value}");
    }
}

#[test]
fn a_toml_date_is_not_a_string() {
    let text = "schema_version = 3\nlocale = 2024-01-01\n";
    let diags = config(text);
    let d = only(&diags);
    assert_eq!(d.code, Code::InvalidType);
    assert!(d.message.contains("found a date-time"), "{}", d.message);

    let text = "schema_version = 3\n\n[terms]\nallow = [1979-05-27]\n";
    let diags = config(text);
    assert_eq!(only(&diags).pointer(), "/terms/allow/0");
}
