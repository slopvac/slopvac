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
mod binary_host;
mod convert;
mod linter;
mod server;

use clap::{ArgMatches, Command};

pub use linter::{Check, EngineLinter, LintError, Linted, Linter, Problem};
pub use server::{Exit, Server, ServerError};

/// How long a clean exit waits for the stdio threads to finish.
const IO_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The `lsp` subcommand.
#[must_use]
pub fn command() -> Command {
    Command::new("lsp").about("Run the language server on standard input and output.")
}

/// Run the language server on standard input and output until the client exits, linting
/// with `linter`.
///
/// The exit status follows the protocol: 0 after `shutdown` and `exit`, 1 for an `exit`
/// with no `shutdown` ([`Exit::WithoutShutdown`]).
pub fn run(_matches: &ArgMatches, linter: EngineLinter) -> Result<Exit, RunError> {
    // STUB(slopvac-8l4.12.15): the caller builds `linter` from the project config found at
    // startup. A document in an `[[overrides]]` block still gets the project settings, and a
    // config change shows only after a restart.
    let (connection, io_threads) = lsp_server::Connection::stdio();
    // STUB(slopvac-8l4.12.16): no `Check` is registered. Front-matter validation
    // (slopvac-8l4.4.6) and the terms check plug in through `Server::with_check`.
    let served = Server::new(std::sync::Arc::new(linter)).serve(&connection);
    drop(connection);
    match served {
        Ok(exit) => {
            // A reading client gets everything flushed. A client that stopped reading
            // blocks the stdout writer for good, so the wait is bounded and the process
            // exits with the threads still blocked.
            let (joined_tx, joined) = crossbeam_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = joined_tx.send(io_threads.join());
            });
            match joined.recv_timeout(IO_WAIT) {
                Ok(Err(e)) => Err(RunError::Io(e.to_string())),
                Ok(Ok(())) | Err(_) => Ok(exit),
            }
        }
        Err(e) => {
            // The reader thread blocks on standard input, which the client may keep open.
            // Joining it would hang the process, so let it go: the process exits anyway.
            drop(io_threads);
            Err(RunError::Server(e))
        }
    }
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
