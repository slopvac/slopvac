//! The `slopvac` command-line interface.
//!
//! The binary is a thin shell over [`main_with`]. Tests and the reference
//! generators build the same tree through [`cli::command`].
#![allow(
    clippy::allow_attributes_without_reason,
    clippy::expect_used,
    reason = "TEMPORARY: burn-down in slopvac-8l4.12.33"
)]

pub mod cli;
pub mod color;
pub mod doctor;
pub mod engine;
pub mod env;
pub mod errors;
pub mod exit;
pub mod generate;
pub mod init;
pub mod inspect;
pub mod lint_messages;
pub mod lint_settings;
pub mod lint_terms;
pub mod logging;
pub mod messages;
mod migrate_lint;
pub mod offline;
pub mod output;
pub mod owners;
pub mod redact;
pub mod report;
pub mod rule_catalog;
mod run;
pub mod severity;
pub mod stdin;
pub mod summary;

pub use env::{Env, SystemEnv};
pub use migrate_lint::MigrateConfigure;
pub use run::{
    Globals, main_with, main_with_io, main_with_stdout, resolve_globals, resolve_globals_with,
};
