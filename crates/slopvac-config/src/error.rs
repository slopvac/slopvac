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
    #[error("cannot read {path}: {}", reason(.source))]
    Io {
        /// The path as the caller gave it, or relative to the project root. Never an absolute
        /// path that the caller did not supply.
        path: String,
        /// The operating-system error, kept for debugging and left out of the message.
        #[source]
        source: io::Error,
    },
    /// A file was read and refused.
    #[error("{}: {}", .0.file, summary(&.0.diagnostics))]
    Rejected(Box<Rejected>),
}

/// A fixed phrase per error kind, so the message does not change with the operating system.
pub(crate) fn reason(e: &io::Error) -> &'static str {
    match e.kind() {
        io::ErrorKind::NotFound => "no such file",
        io::ErrorKind::PermissionDenied => "permission denied",
        io::ErrorKind::InvalidData => "not valid UTF-8",
        _ => "the file cannot be read",
    }
}

fn summary(diagnostics: &[Diagnostic]) -> String {
    match diagnostics {
        [only] => only.to_string(),
        many => format!("{} problems", many.len()),
    }
}

impl ConfigError {
    /// An I/O failure on `label`, the path as the user would recognise it.
    #[must_use]
    pub fn io(label: impl Into<String>, source: io::Error) -> Self {
        ConfigError::Io {
            path: label.into(),
            source,
        }
    }

    /// The stable error class: the first diagnostic's code, or `config-io` for a read failure.
    #[must_use]
    pub fn code(&self) -> Code {
        match self {
            ConfigError::Io { .. } => Code::Io,
            ConfigError::Rejected(r) => r.diagnostics.first().map_or(Code::Parse, |d| d.code),
        }
    }

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
            ConfigError::Io { .. } => format!("error[{}]: {self}", Code::Io.as_str()),
            ConfigError::Rejected(r) => r.render(),
        }
    }
}
