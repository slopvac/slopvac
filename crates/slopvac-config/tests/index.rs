//! Rules for `[index]` that JSON Schema cannot express, and edge cases of the extra-index source.

use rstest::rstest;
use slopvac_config::tree::parse_toml;
use slopvac_config::{Code, validate_config};

fn check(index: &str) -> Vec<slopvac_config::Diagnostic> {
    let text = format!("schema_version = 3\n\n{index}");
    validate_config(&parse_toml(&text).unwrap(), &text)
}

fn extra(name: &str, source: &str) -> String {
    format!("[[index.extra]]\nname = \"{name}\"\nsource = \"{source}\"\n")
}

#[test]
fn an_extra_index_name_listed_twice_is_an_error_at_the_second_entry() {
    let text = format!(
        "{}\n{}",
        extra("acme", "https://plugins.acme.example/index.json"),
        extra("acme", "oci://ghcr.io/acme/index")
    );
    let diags = check(&text);
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].code, Code::InvalidValue);
    assert_eq!(diags[0].pointer(), "/index/extra/1/name");
}

#[test]
fn two_extra_indexes_with_different_names_and_the_default_switch_are_valid() {
    let text = format!(
        "[index]\ndefault = false\n\n{}\n{}",
        extra("acme", "https://plugins.acme.example/index.json"),
        extra("mirror", "oci://ghcr.io/acme/index")
    );
    assert!(check(&text).is_empty());
}

#[rstest]
#[case::https("https://plugins.acme.example/index.json")]
#[case::oci("oci://ghcr.io/acme/index")]
#[case::port("https://localhost:8443/index.json")]
#[case::bare_host("https://plugins.acme.example")]
#[case::query_with_at("https://plugins.acme.example/index.json?ref=a@b")]
fn an_extra_index_source_that_names_a_host_is_valid(#[case] source: &str) {
    assert!(check(&extra("acme", source)).is_empty());
}

#[rstest]
#[case::no_host_https("https://")]
#[case::no_host_oci("oci://")]
#[case::path_as_host("https:///index.json")]
#[case::user_in_authority("https://reader@plugins.acme.example/index.json")]
#[case::empty_user_in_authority("oci://@ghcr.io/acme/index")]
#[case::space("https://plugins.acme.example/in dex.json")]
#[case::other_scheme("ftp://plugins.acme.example/index.json")]
#[case::upper_scheme("HTTPS://plugins.acme.example/index.json")]
fn an_extra_index_source_without_a_clean_host_is_invalid(#[case] source: &str) {
    let diags = check(&extra("acme", source));
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].pointer(), "/index/extra/0/source");
}

#[rstest]
#[case::at_cap(2048, true)]
#[case::past_cap(2049, false)]
fn an_extra_index_source_is_capped_at_2048_characters(#[case] length: usize, #[case] valid: bool) {
    let prefix = "https://plugins.acme.example/";
    let source = format!("{prefix}{}", "a".repeat(length - prefix.len()));
    assert_eq!(check(&extra("acme", &source)).is_empty(), valid);
}
