//! The language server's error messages and registry entries.

use std::error::Error as _;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use rstest::rstest;
use serde_json::json;
use slopvac_lsp::{Exit, LintError, Linted, Linter, RunError, Server, ServerError};
use slopvac_messages::Catalog as _;
use slopvac_model::RuleId;

const WAIT: Duration = Duration::from_secs(20);

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

/// A linter that never runs: the server fails before it lints anything.
struct NoLint;

impl Linter for NoLint {
    fn lint(&self, _path: &str, _text: &str) -> Result<Linted, LintError> {
        unreachable!("the server fails before it lints");
    }

    fn allow_reasons(&self, _: &str, _: &str, _: usize, _: &RuleId) -> Vec<String> {
        Vec::new()
    }
}

/// A string no message of the server may contain; it stands for document text or a path.
const MARKER: &str = "SECRET-MARKER-3f9a";

/// Serve one in-memory connection and return what the client side can send through.
fn spawn_server() -> (Connection, JoinHandle<Result<Exit, ServerError>>) {
    let (server_side, client) = Connection::memory();
    let server = Server::new(Arc::new(NoLint));
    let handle = std::thread::spawn(move || server.serve(&server_side));
    (client, handle)
}

#[test]
fn a_protocol_error_does_not_carry_document_text() {
    let (client, handle) = spawn_server();
    let send = |message: Message| client.sender.send(message).unwrap();
    send(Message::Request(Request::new(
        RequestId::from(1),
        "initialize".to_owned(),
        json!({"processId": null, "rootUri": null, "capabilities": {}}),
    )));
    client.receiver.recv_timeout(WAIT).unwrap();
    send(Message::Notification(Notification::new(
        "initialized".to_owned(),
        json!({}),
    )));
    send(Message::Request(Request::new(
        RequestId::from(2),
        "shutdown".to_owned(),
        json!(null),
    )));
    // Anything but `exit` after `shutdown` is a protocol error that quotes this message.
    send(Message::Notification(Notification::new(
        "textDocument/didOpen".to_owned(),
        json!({"textDocument": {"uri": "file:///a.md", "languageId": "markdown",
                                "version": 1, "text": MARKER}}),
    )));
    let error = handle.join().unwrap().unwrap_err();
    assert_eq!(error.code(), "lsp-protocol-error");
    assert!(!error.to_string().contains(MARKER), "{error}");
    assert!(!format!("{error:?}").contains(MARKER), "{error:?}");
    assert!(error.source().is_none());
}

#[test]
fn an_initialize_error_does_not_carry_the_rejected_value() {
    let (client, handle) = spawn_server();
    client
        .sender
        .send(Message::Request(Request::new(
            RequestId::from(1),
            "initialize".to_owned(),
            json!({"processId": MARKER, "rootUri": null, "capabilities": {}}),
        )))
        .unwrap();
    let error = handle.join().unwrap().unwrap_err();
    assert_eq!(error.code(), "lsp-initialize-invalid");
    // The parameters are a parsed value, so serde_json has no text position to give.
    assert_eq!(
        error.to_string(),
        "invalid initialize parameters: data error at line 0, column 0"
    );
    assert!(!error.to_string().contains(MARKER), "{error}");
    assert!(!format!("{error:?}").contains(MARKER), "{error:?}");
    assert!(error.source().is_none());
}
