//! Errors from loading configuration.

use std::io;

use crate::diag::{Code, Diagnostic, SourceFile, render};

/// A file whose text slopvac refused, with the reasons.
#[derive(Debug, Clone, PartialEq)]
pub struct Rejected {
    /// Path shown in messages, relative to the project root.
    pub file: String,
    /// The full text the diagnostic spans index into.
    pub text: String,
    /// What is wrong, in document order.
    pub diagnostics: Vec<Diagnostic>,
}

impl Rejected {
    /// Render every diagnostic as plain text.
    #[must_use]
    pub fn render(&self) -> String {
        render(
            SourceFile {
                name: &self.file,
                text: &self.text,
            },
            &self.diagnostics,
        )
    }
}

/// Why configuration could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The path that failed.
        path: String,
        /// The operating-system error.
        #[source]
        source: io::Error,
    },
    /// A file was read and refused.
    #[error("{}: {}", .0.file, summary(&.0.diagnostics))]
    Rejected(Box<Rejected>),
}

fn summary(diagnostics: &[Diagnostic]) -> String {
    match diagnostics {
        [only] => only.to_string(),
        many => format!("{} problems", many.len()),
    }
}

impl ConfigError {
    /// Process exit code for this error. Config errors never lint anything, so they are 2.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        2
    }

    /// Whether the file is a 2.x config that `slopvac migrate` can convert.
    #[must_use]
    pub fn is_legacy(&self) -> bool {
        matches!(self, ConfigError::Rejected(r) if r.diagnostics.iter().any(|d| d.code == Code::LegacyVersion))
    }

    /// The error as text a user can act on: rendered diagnostics, or the I/O message.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            ConfigError::Io { .. } => self.to_string(),
            ConfigError::Rejected(r) => r.render(),
        }
    }
}
