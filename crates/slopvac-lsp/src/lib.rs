//! Language server for editor integration.
//!
//! [`Server`] speaks LSP over a [`lsp_server::Connection`]. It publishes diagnostics when a
//! document opens, changes or saves, offers code actions to apply a finding's fix or to
//! insert an allow directive with a reason, and explains a finding on hover. It is
//! synchronous: one reader thread, one lint worker thread and `crossbeam-channel`.
//!
//! The server asks the engine through [`Linter`], and runs extra [`Check`]s next to it
//! (the terms check and front-matter validation plug in there). Position units follow
//! `docs/contract/positions.md`: the server negotiates `utf-8`, `utf-32` or `utf-16` with
//! [`slopvac_model::PositionEncoding::negotiate`] and derives every range from the buffer
//! text it received.
//!
//! [`command`] and [`run`] are the `slopvac lsp` subcommand.

mod analysis;
mod convert;
mod linter;
mod server;

use clap::{ArgMatches, Command};
use slopvac_engine::Settings;
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString};

pub use linter::{Check, EngineLinter, LintError, Linter, Problem};
pub use server::{Server, ServerError};

/// The `lsp` subcommand.
#[must_use]
pub fn command() -> Command {
    Command::new("lsp").about("Run the language server on standard input and output.")
}

/// Run the language server on standard input and output until the client exits.
pub fn run(_matches: &ArgMatches) -> Result<(), RunError> {
    // STUB(slopvac-8l4.12.15): no rules are loaded. The server needs the resolved rule sets
    // and settings from the config layer; until then it reports no findings.
    let settings = Settings::new(
        NonEmptyString::new("normal").map_err(|e| RunError::Setup(e.to_string()))?,
        GenreResult {
            id: GenreId::new("reference").map_err(|e| RunError::Setup(e.to_string()))?,
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    let linter =
        EngineLinter::new(Vec::new(), settings).map_err(|e| RunError::Setup(e.to_string()))?;
    let (connection, io_threads) = lsp_server::Connection::stdio();
    // STUB(slopvac-8l4.12.16): no `Check` is registered. Front-matter validation
    // (slopvac-8l4.4.6) and the terms check plug in through `Server::with_check`.
    let served = Server::new(std::sync::Arc::new(linter)).serve(&connection);
    drop(connection);
    io_threads.join().map_err(|e| RunError::Io(e.to_string()))?;
    served.map_err(RunError::Server)
}

/// Why `slopvac lsp` stopped with an error.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The engine could not be built.
    #[error("cannot start the language server: {0}")]
    Setup(String),
    /// The standard streams failed.
    #[error("language server input or output failed: {0}")]
    Io(String),
    /// The server ended in error.
    #[error(transparent)]
    Server(ServerError),
}
