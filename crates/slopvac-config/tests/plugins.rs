//! Rules for the `[[plugins]]` list that JSON Schema cannot express, so the corpus has no case.

use slopvac_config::tree::parse_toml;
use slopvac_config::{Code, validate_config};

#[test]
fn a_plugin_id_listed_twice_is_an_error_at_the_second_entry() {
    let text = "schema_version = 3\n\n[[plugins]]\nid = \"acme\"\nsource = \"oci://ghcr.io/acme/style\"\n\n[[plugins]]\nid = \"acme\"\nsource = \"../plugins/acme\"\n";
    let diags = validate_config(&parse_toml(text).unwrap(), text);
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].code, Code::InvalidValue);
    assert_eq!(diags[0].pointer(), "/plugins/1/id");
}
