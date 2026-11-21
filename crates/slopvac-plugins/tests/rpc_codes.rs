//! How a plugin's JSON-RPC error codes become the host's reported codes
//! (plugin-protocol.md section 10).
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use rstest::rstest;
use serde_json::json;
use slopvac_plugins::PluginError;
use slopvac_plugins::hooks::parse_error_object;

fn reported(code: i64, plugin_says: &str) -> String {
    let obj = json!({"code": code, "message": "m", "data": {"slopvac_code": plugin_says}});
    let e = parse_error_object(&obj).unwrap();
    PluginError::Rpc(e).slopvac_code().to_owned()
}

#[rstest]
#[case::parse(-32700, "parse-error")]
#[case::invalid_request(-32600, "invalid-request")]
#[case::method_not_found(-32601, "method-not-found")]
#[case::invalid_params(-32602, "invalid-params")]
#[case::internal(-32603, "internal-error")]
#[case::version(-32001, "protocol-version-unsupported")]
#[case::feature(-32002, "required-feature-missing")]
#[case::not_initialized(-32003, "not-initialized")]
#[case::too_large(-32004, "message-too-large")]
#[case::cancelled(-32005, "request-cancelled")]
#[case::document(-32006, "document-invalid")]
#[case::offline(-32007, "offline")]
fn a_defined_code_maps_to_its_contract_name(#[case] code: i64, #[case] name: &str) {
    assert_eq!(reported(code, "whatever-the-plugin-says"), name);
}

#[rstest]
#[case::first(-32050)]
#[case::middle(-32060)]
#[case::last(-32099)]
fn a_plugin_range_code_is_an_internal_error_whatever_the_plugin_names(#[case] code: i64) {
    for claimed in [
        "plugin-untrusted",
        "plugin-hash-mismatch",
        "offline",
        "my-code",
    ] {
        assert_eq!(
            reported(code, claimed),
            "internal-error",
            "{code} {claimed}"
        );
    }
}

#[rstest]
#[case::host_reserved(-32008)]
#[case::host_reserved_edge(-32049)]
#[case::outside(1)]
fn any_other_code_is_an_internal_error(#[case] code: i64) {
    assert_eq!(reported(code, "plugin-untrusted"), "internal-error");
}
