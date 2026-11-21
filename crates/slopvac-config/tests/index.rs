//! Rules for `[index]` that JSON Schema cannot express, and edge cases of the extra-index source.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

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
    assert_eq!(check(&text), Vec::new());
}

#[rstest]
#[case::https("https://plugins.acme.example/index.json")]
#[case::oci("oci://ghcr.io/acme/index")]
#[case::port("https://localhost:8443/index.json")]
#[case::bare_host("https://plugins.acme.example")]
#[case::query_with_at("https://plugins.acme.example/index.json?ref=a@b")]
fn an_extra_index_source_that_names_a_host_is_valid(#[case] source: &str) {
    assert_eq!(check(&extra("acme", source)), Vec::new());
}

#[rstest]
#[case::no_host_https("https://", "it names no host")]
#[case::no_host_oci("oci://", "it names no host")]
#[case::path_as_host("https:///index.json", "it names no host")]
#[case::user_in_authority(
    "https://reader@plugins.acme.example/index.json",
    "it carries user-info"
)]
#[case::empty_user_in_authority("oci://@ghcr.io/acme/index", "it carries user-info")]
#[case::space("https://plugins.acme.example/in dex.json", "it holds whitespace")]
#[case::other_scheme(
    "ftp://plugins.acme.example/index.json",
    "its scheme is not `oci://` or `https://`"
)]
#[case::upper_scheme(
    "HTTPS://plugins.acme.example/index.json",
    "its scheme is not `oci://` or `https://`"
)]
fn an_extra_index_source_without_a_clean_host_is_refused_with_its_reason(
    #[case] source: &str,
    #[case] reason: &str,
) {
    let diags = check(&extra("acme", source));
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].pointer(), "/index/extra/0/source");
    assert_eq!(
        diags[0].message,
        format!("`index.extra[0].source` is refused: {reason}")
    );
}

#[test]
fn a_refused_source_never_shows_the_credential_it_carries() {
    let sentinel = "SECRET";
    let sources = [
        format!("https://user:{sentinel}@plugins.acme.example/index.json"),
        format!("oci://user:{sentinel}@ghcr.io/acme/index"),
        format!("ftp://user:{sentinel}@plugins.acme.example/index.json"),
        format!("https://plugins.acme.example/in dex?token={sentinel}"),
        format!(
            "https://plugins.acme.example/{}?token={sentinel}",
            "a".repeat(2100)
        ),
    ];
    for source in sources {
        let diags = check(&extra("acme", &source));
        assert_eq!(diags.len(), 1, "{source}: {diags:#?}");
        let d = &diags[0];
        let shown = [
            d.to_string(),
            format!("{d:?}"),
            d.message.clone(),
            d.label.clone(),
            d.help.clone().unwrap_or_default(),
        ];
        for text in shown {
            assert!(!text.contains(sentinel), "{text}");
        }
    }
}

#[rstest]
#[case::at_cap(2048, true)]
#[case::past_cap(2049, false)]
fn an_extra_index_source_is_capped_at_2048_characters(#[case] length: usize, #[case] valid: bool) {
    let prefix = "https://plugins.acme.example/";
    let source = format!("{prefix}{}", "a".repeat(length - prefix.len()));
    assert_eq!(check(&extra("acme", &source)).is_empty(), valid);
}
