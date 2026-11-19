//! Every code and user-facing text of the language server, as message catalogs.

use std::sync::LazyLock;

use slopvac_messages::{Catalog, Entry};

/// A failure that ends the server.
#[derive(Debug, Catalog)]
pub enum ServerError {
    /// The connection broke or the client broke the protocol.
    #[code("lsp-protocol-error")]
    #[message("language server protocol error")]
    Protocol {
        /// The protocol failure.
        #[from]
        source: lsp_server::ProtocolError,
    },
    /// The client sent `initialize` parameters this server cannot read.
    #[code("lsp-initialize-invalid")]
    #[message("invalid initialize parameters")]
    Initialize {
        /// The parse failure.
        #[from]
        source: serde_json::Error,
    },
    /// The connection closed while the server was still sending.
    #[code("lsp-connection-closed")]
    #[message("the client connection closed")]
    Closed,
    /// The lint worker thread panicked or could not start.
    #[code("lsp-worker-panicked")]
    #[message("the lint worker panicked")]
    WorkerPanicked,
}

/// Why `slopvac lsp` stopped with an error.
#[derive(Debug, Catalog)]
pub enum RunError {
    /// The engine could not be built.
    #[code("lsp-setup-failed")]
    #[message("cannot start the language server: {detail}")]
    Setup {
        /// What went wrong.
        detail: String,
    },
    /// The standard streams failed.
    #[code("lsp-io-failed")]
    #[message("language server input or output failed: {detail}")]
    Io {
        /// What went wrong.
        detail: String,
    },
    /// The server ended in error.
    #[code(delegate)]
    Server {
        /// The server failure.
        #[from]
        source: ServerError,
    },
}

/// Text the server shows in the editor: diagnostics, hovers, code action titles, request
/// errors and help.
#[derive(Debug, Catalog)]
pub(crate) enum LspMessage {
    /// Help for the `lsp` subcommand.
    #[code("lsp-help-about")]
    #[text("Run the language server on standard input and output.")]
    HelpAbout,
    /// A binary container is not linted in the editor.
    #[code("binary-host-unchecked")]
    #[notice("binary host not linted in the editor; run the CLI")]
    BinaryHostUnchecked,
    /// The lint could not finish, so the buffer must not look clean.
    #[code("lint-incomplete")]
    #[notice("slopvac could not lint this document: {reason}")]
    LintIncomplete { reason: String },
    /// Part of the check did not run.
    #[code("lsp-unchecked")]
    #[notice("slopvac did not fully check this document: {message}")]
    Unchecked { message: String },
    /// Hover heading for a finding. The message is rule data.
    #[code("lsp-hover-finding")]
    #[text("**{rule}** ({severity}, category `{category}`)\n\n{message}")]
    HoverFinding {
        rule: String,
        severity: String,
        category: String,
        message: String,
    },
    /// Hover line that shows the matched text.
    #[code("lsp-hover-matched")]
    #[text("Matched: `{text}`")]
    HoverMatched { text: String },
    /// Hover line that lists the replacement texts of a fix.
    #[code("lsp-hover-fix")]
    #[text("Fix: replace with {replacements}")]
    HoverFix { replacements: String },
    /// Hover heading for a problem from an extra check.
    #[code("lsp-hover-problem")]
    #[text("**{code}**\n\n{message}")]
    HoverProblem { code: String, message: String },
    /// Code action title that applies a finding's fix.
    #[code("lsp-action-fix")]
    #[text("Apply slopvac fix: {code}")]
    ActionFix { code: String },
    /// Code action title that inserts an allow directive.
    #[code("lsp-action-allow")]
    #[text("Allow {rule} (reason: {reason})")]
    ActionAllow { rule: String, reason: String },
    /// The client sent a request the server does not handle.
    #[code("lsp-method-unsupported")]
    #[message("method not supported: {method}")]
    MethodUnsupported { method: String },
}

/// Every entry of the language server's catalogs.
#[must_use]
pub fn entries() -> &'static [Entry] {
    static ENTRIES: LazyLock<Vec<Entry>> = LazyLock::new(|| {
        [
            ServerError::entries(),
            RunError::entries(),
            LspMessage::entries(),
        ]
        .concat()
    });
    &ENTRIES
}
