//! Agent guidance for slopvac: prime topics, harness setup and review.
//!
//! The CLI crate mounts each subcommand through its `command()` and `run()`.

pub mod block;
pub mod content;
pub mod error;
pub mod fsx;
pub mod harness;
pub mod managed;
pub mod setup;

pub use error::AgentsError;
