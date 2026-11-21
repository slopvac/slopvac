//! Language server for editor integration.
//!
//! [`Server`] speaks LSP over a [`lsp_server::Connection`]. It publishes diagnostics when a
//! document opens, changes or saves, offers code actions to apply a finding's fix or to
//! insert an allow directive with a reason, and explains a finding on hover. It is
//! synchronous: one reader thread, one lint worker thread and `crossbeam-channel`.
//!
//! The server asks the engine through [`Linter`], and runs extra [`Check`]s next to it.
//! Term casing comes from the engine. Only front-matter validation plugs in as a [`Check`].
//! Position units follow `docs/contract/positions.md`: the server negotiates `utf-8`, `utf-32` or `utf-16` with
//! [`slopvac_model::PositionEncoding::negotiate`] and derives every range from the buffer
//! text it received.
//!
//! [`command`] and [`run`] are the `slopvac lsp` subcommand.

mod analysis;
mod binary_host;
mod checks;
mod convert;
mod linter;
mod messages;
mod server;

use clap::{ArgMatches, Command};
use slopvac_messages::render;

use messages::LspMessage;

pub use checks::FrontMatterCheck;
pub use linter::{
    Check, ConfigFailure, EngineLinter, LintError, Linted, Linter, Problem, Reloaded,
    exception_names,
};
pub use messages::{RunError, ServerError, entries as message_entries};
pub use server::{Exit, Server};

/// How long a clean exit waits for the stdio threads to finish.
const IO_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The `lsp` subcommand.
#[must_use]
pub fn command() -> Command {
    Command::new("lsp").about(render(&LspMessage::HelpAbout))
}

/// Run the language server on standard input and output until the client exits, linting
/// with `linter` and running each of `checks` next to it. The caller builds the checks from
/// the project config, as it does `linter`. The client sees `startup_notices` once, as
/// warnings, right after the handshake.
///
/// The exit status follows the protocol: 0 after `shutdown` and `exit`, 1 for an `exit`
/// with no `shutdown` ([`Exit::WithoutShutdown`]).
pub fn run(
    _matches: &ArgMatches,
    linter: EngineLinter,
    startup_notices: Vec<slopvac_model::Notice>,
    checks: Vec<std::sync::Arc<dyn Check>>,
) -> Result<Exit, RunError> {
    // STUB(slopvac-8l4.12.15): the caller builds `linter` from the project config found at
    // startup. A document in an `[[overrides]]` block still gets the project settings, and a
    // config change shows only after a restart.
    let (connection, io_threads) = lsp_server::Connection::stdio();
    let configured = checks
        .into_iter()
        .fold(Server::new(std::sync::Arc::new(linter)), Server::with_check);
    let served = configured
        .with_startup_notices(startup_notices)
        .serve(&connection);
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
                // The error can wrap a protocol parse failure that quotes client text, so
                // only its kind is reported.
                Ok(Err(e)) => Err(RunError::Io {
                    detail: e.kind().to_string(),
                }),
                Ok(Ok(())) | Err(_) => Ok(exit),
            }
        }
        Err(e) => {
            // The reader thread blocks on standard input, which the client may keep open.
            // Joining it would hang the process, so let it go: the process exits anyway.
            drop(io_threads);
            Err(RunError::Server { source: e })
        }
    }
}
