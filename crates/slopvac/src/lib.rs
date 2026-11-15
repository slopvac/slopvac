//! The `slopvac` command-line interface.
//!
//! The binary is a thin shell over [`main_with`]. Tests and the reference
//! generators build the same tree through [`cli::command`].

pub mod cli;
pub mod color;
pub mod engine_stub;
pub mod env;
pub mod errors;
pub mod exit;
pub mod generate;
pub mod logging;
pub mod offline;
pub mod redact;
mod run;

pub use env::{Env, SystemEnv};
pub use run::{Globals, main_with, main_with_io, main_with_stdout, resolve_globals};
