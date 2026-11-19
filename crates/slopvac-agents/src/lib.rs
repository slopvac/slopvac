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
pub mod reviewers;
pub mod setup;

pub use error::AgentsError;

#[cfg(feature = "test-counters")]
#[doc(hidden)]
pub use block::marker_scan_steps;
