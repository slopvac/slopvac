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
//! A [`Linter`] can read its config again: the server calls [`Linter::reload`] when a config
//! file is saved or changes on disk, shows the config warnings, shows a config that fails to
//! load on the file that failed, and lints every open document again. A config that fails
//! to load at startup is shown the same way ([`Server::with_startup_failure`]). It asks the
//! client to watch the config files when the client can.
//!
//! [`command`] and [`run`] are the `slopvac lsp` subcommand.

mod analysis;
mod binary_host;
mod checks;
mod convert;
mod linter;
mod messages;
mod server;
mod uri;

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

/// Run `server` on standard input and output until the client exits. The caller builds it
/// from the project config: the linter, the checks it runs next to the linter, and the
/// notices and the config failure the client sees right after the handshake.
///
/// The exit status follows the protocol: 0 after `shutdown` and `exit`, 1 for an `exit`
/// with no `shutdown` ([`Exit::WithoutShutdown`]).
pub fn run(_matches: &ArgMatches, server: Server) -> Result<Exit, RunError> {
    let (connection, io_threads) = lsp_server::Connection::stdio();
    let served = server.serve(&connection);
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
