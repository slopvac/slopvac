//! Config refuses the reserved plugin names. Expected values are written out, not read from the
//! constants, so a change to the reserved set fails here.

use rstest::rstest;
use slopvac_config::tree::parse_toml;
use slopvac_config::{Code, validate_config};

#[expect(
    clippy::unwrap_used,
    reason = "test input is valid TOML; a panic is a failed test"
)]
fn diagnostics(body: &str) -> Vec<slopvac_config::Diagnostic> {
    let text = format!("schema_version = 3\n\n{body}");
    validate_config(&parse_toml(&text).unwrap(), &text)
}

#[test]
fn a_plugin_entry_with_the_id_core_is_an_invalid_value_at_its_id() {
    let diags = diagnostics("[[plugins]]\nid = \"core\"\nsource = \"../plugins/x\"\n");
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].code, Code::InvalidValue);
    assert_eq!(diags[0].pointer(), "/plugins/0/id");
}

#[test]
fn a_plugin_entry_with_an_ordinary_id_is_clean() {
    let diags = diagnostics("[[plugins]]\nid = \"acme\"\nsource = \"../plugins/x\"\n");
    assert!(diags.is_empty(), "{diags:#?}");
}

#[rstest]
#[case::built_in("core")]
#[case::first_party("slopvac")]
fn an_extra_index_may_not_take_a_reserved_name(#[case] name: &str) {
    let body = format!(
        "[index]\n\n[[index.extra]]\nname = \"{name}\"\nsource = \"https://example.com/index\"\n"
    );
    let diags = diagnostics(&body);
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].code, Code::InvalidValue);
    assert_eq!(diags[0].pointer(), "/index/extra/0/name");
}

#[test]
fn an_extra_index_with_an_ordinary_name_is_clean() {
    let body =
        "[index]\n\n[[index.extra]]\nname = \"acme\"\nsource = \"https://example.com/index\"\n";
    let diags = diagnostics(body);
    assert!(diags.is_empty(), "{diags:#?}");
}
