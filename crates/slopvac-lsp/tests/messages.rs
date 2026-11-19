//! The language server's error messages and registry entries.

use std::error::Error as _;

use rstest::rstest;
use slopvac_lsp::{RunError, ServerError};
use slopvac_messages::Catalog as _;

#[rstest]
#[case::closed(
    ServerError::Closed,
    "lsp-connection-closed",
    "the client connection closed"
)]
#[case::worker(
    ServerError::WorkerPanicked,
    "lsp-worker-panicked",
    "the lint worker panicked"
)]
fn server_errors_carry_a_code_and_text(
    #[case] error: ServerError,
    #[case] code: &str,
    #[case] text: &str,
) {
    assert_eq!(error.code(), code);
    assert_eq!(error.to_string(), text);
}

#[test]
fn a_run_error_renders_its_detail() {
    let error = RunError::Io {
        detail: "broken pipe".to_owned(),
    };
    assert_eq!(error.code(), "lsp-io-failed");
    assert_eq!(
        error.to_string(),
        "language server input or output failed: broken pipe"
    );
}

#[test]
fn a_wrapped_server_error_keeps_its_text_and_code() {
    let error = RunError::from(ServerError::Closed);
    assert_eq!(error.to_string(), "the client connection closed");
    assert_eq!(error.code(), "lsp-connection-closed");
    assert!(error.source().is_none());
}
