//! Agent guidance for slopvac: prime topics, harness setup and review.
//!
//! The CLI crate mounts each subcommand through its `command()` and `run()`.
#![allow(
    clippy::unwrap_used,
    reason = "TEMPORARY: burn-down in slopvac-8l4.12.33"
)]

pub mod block;
pub mod claims;
pub mod content;
pub mod error;
pub mod fsx;
pub mod guidance;
pub mod harness;
pub mod managed;
mod messages;
pub use messages::message_entries;
pub mod models;
pub mod prime;
pub mod review;
pub mod reviewers;
mod schema;
pub mod setup;

pub use error::AgentsError;

#[cfg(feature = "test-counters")]
#[doc(hidden)]
pub use block::marker_scan_steps;
