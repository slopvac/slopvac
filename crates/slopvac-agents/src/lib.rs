//! Agent guidance for slopvac: prime topics, harness setup and review.
//!
//! The CLI crate mounts each subcommand through its `command()` and `run()`.

pub mod block;
pub mod claims;
pub mod content;
pub mod error;
pub mod fsx;
pub mod guidance;
pub mod harness;
pub mod managed;
pub mod models;
pub mod prime;
pub mod review;
pub mod setup;

pub use error::AgentsError;

/// Drop the result of a write into a `String`. `fmt::Write for String` never returns an
/// error, so there is nothing to handle; this is the one place that says so.
#[expect(
    clippy::let_underscore_must_use,
    reason = "fmt::Write for String never returns an error"
)]
pub(crate) fn infallible(result: std::fmt::Result) {
    let _ = result;
}
