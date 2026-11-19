//! Catalogs shared by the integration tests.

#![allow(
    dead_code,
    unreachable_pub,
    reason = "test-support module: each test binary uses part of it"
)]

use std::path::PathBuf;

use slopvac_messages::Catalog;

/// Errors of a made-up reader.
#[derive(Debug, Catalog)]
pub enum ReadError {
    /// The file could not be read.
    #[code("file-unreadable")]
    #[message("cannot read {path}: {reason}")]
    Unreadable {
        #[display_path]
        path: PathBuf,
        reason: String,
        #[source]
        cause: std::io::Error,
    },
    /// A directory held too many files.
    #[code("too-many-files")]
    #[message("found {found} files, the limit is {limit}")]
    TooMany {
        #[count]
        found: usize,
        #[count]
        limit: u32,
    },
    /// The config text uses braces.
    #[code("brace-example")]
    #[message("write {{key}} to name {key}")]
    Braces { key: String },
    /// A line break in the middle.
    #[code("two-lines")]
    #[message("first line\nsecond {key}\n\n.dotted line")]
    TwoLines { key: String },
}

/// Errors that wrap another catalog.
#[derive(Debug, Catalog)]
pub enum RunError {
    /// Reading failed.
    #[code(delegate)]
    Read(#[from] ReadError),
    /// The run was cancelled.
    #[code("run-cancelled")]
    #[message("cancelled")]
    Cancelled,
    /// A named wrapper.
    #[code(delegate)]
    Named {
        #[source]
        inner: ReadError,
    },
}

/// Help texts with no parameters.
#[derive(Debug, Catalog)]
pub enum Help {
    /// The `--check` flag.
    #[code("help-check")]
    #[text("Fail when a finding remains")]
    Check,
    /// A notice with braces.
    #[code("help-braces")]
    #[notice("Use {{name}} placeholders")]
    Braces {},
}

/// A variant whose cause is a field named `source` with no attribute.
#[derive(Debug, Catalog)]
pub enum Implicit {
    /// The step failed.
    #[code("step-failed")]
    #[message("step {step} failed")]
    Step {
        step: String,
        source: std::io::Error,
    },
}
