//! Errors from loading configuration.

use std::io;
use std::path::PathBuf;

use slopvac_messages::Catalog;

use crate::diag::{Code, Diagnostic, SourceFile, render_config};
use crate::messages::{ConfigMessage, ConfigPhrase, ConfigText};

/// A file whose text slopvac refused, with the reasons.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Rejected {
    /// Path shown in messages, relative to the project root.
    pub file: String,
    /// The file itself, which `file` hides when it sits outside the project root. It is for
    /// finding the file again and never appears in a message.
    pub path: Option<PathBuf>,
    /// Every file the load reached before it stopped, the refused one and an extended file
    /// that does not exist included. See [`ConfigError::dependencies`].
    pub reads: Vec<PathBuf>,
    /// The full text the diagnostic spans index into.
    pub text: String,
    /// What is wrong, in document order.
    pub diagnostics: Vec<Diagnostic>,
}

impl Rejected {
    /// Render every diagnostic as plain text.
    #[must_use]
    pub fn render(&self) -> String {
        render_config(
            SourceFile {
                name: &self.file,
                text: &self.text,
            },
            &self.diagnostics,
        )
    }
}

/// Why configuration could not be loaded.
///
/// Each variant's text comes from the message catalog through the helpers below.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A file could not be read.
    #[non_exhaustive]
    #[error("{}", io_text(.path, .source))]
    Io {
        /// The path as the caller gave it, or relative to the project root. Never an absolute
        /// path that the caller did not supply.
        path: String,
        /// The operating-system error, kept for debugging and left out of the message.
        #[source]
        source: io::Error,
        /// The file itself, when the caller knows it. It is for finding the file again and
        /// never appears in the message.
        file: Option<PathBuf>,
    },
    /// A config file is larger than [`crate::MAX_CONFIG_BYTES`].
    #[non_exhaustive]
    #[error("{}", too_large_text(.path, *.cap))]
    TooLarge {
        /// The path as the caller gave it, or relative to the project root.
        path: String,
        /// The cap that the file exceeded, in bytes.
        cap: u64,
        /// The file itself, as for [`ConfigError::Io`].
        file: Option<PathBuf>,
    },
    /// A file was read and refused.
    #[error("{}", summary(.0))]
    Rejected(Box<Rejected>),
}

fn io_text(path: &str, source: &io::Error) -> String {
    ConfigMessage::Io {
        path: path.to_owned(),
        reason: reason(source).to_owned(),
    }
    .to_string()
}

fn too_large_text(path: &str, cap: u64) -> String {
    ConfigMessage::TooLarge {
        path: path.to_owned(),
        cap,
    }
    .to_string()
}

/// A fixed phrase per error kind, so the message does not change with the operating system.
pub(crate) fn reason(e: &io::Error) -> &'static str {
    match e.kind() {
        io::ErrorKind::NotFound => ConfigPhrase::ReasonNotFound,
        io::ErrorKind::PermissionDenied => ConfigPhrase::ReasonPermissionDenied,
        io::ErrorKind::InvalidData => ConfigPhrase::ReasonNotUtf8,
        _ => ConfigPhrase::ReasonUnreadable,
    }
    .as_static()
}

fn summary(rejected: &Rejected) -> String {
    let file = rejected.file.clone();
    match rejected.diagnostics.as_slice() {
        [only] => ConfigText::RejectedOne {
            file,
            diagnostic: only.to_string(),
        }
        .to_string(),
        many => ConfigText::RejectedMany {
            file,
            problems: many.len(),
        }
        .to_string(),
    }
}

/// One `error[code]: message` line, the form every non-snippet report uses.
pub(crate) fn error_line(code: Code, message: &dyn Catalog) -> String {
    format!(
        "error[{}]: {}",
        code.as_str(),
        slopvac_messages::render(message)
    )
}

impl ConfigError {
    /// An I/O failure on `label`, the path as the user would recognise it.
    #[must_use]
    pub fn io(label: impl Into<String>, source: io::Error) -> Self {
        ConfigError::Io {
            path: label.into(),
            source,
            file: None,
        }
    }

    /// The file this error is about, when it is known. A refusal carries the file it
    /// refused, and a read failure the file it could not read. The shown path hides the
    /// directory of a file outside the project root, so use this one to find the file.
    #[must_use]
    pub fn source_file(&self) -> Option<&std::path::Path> {
        match self {
            ConfigError::Io { file, .. } | ConfigError::TooLarge { file, .. } => file.as_deref(),
            ConfigError::Rejected(rejected) => rejected.path.as_deref(),
        }
    }

    /// The files a load reached before it failed, in the order it reached them: the config
    /// itself, the extended files read so far, the one that failed, and an extended file that
    /// does not exist. They are absolute and lexically normalised, so a watcher can wait for
    /// any of them to change and try again. A read failure names its own file.
    #[must_use]
    pub fn dependencies(&self) -> &[PathBuf] {
        match self {
            ConfigError::Io { file, .. } | ConfigError::TooLarge { file, .. } => file.as_slice(),
            ConfigError::Rejected(rejected) => &rejected.reads,
        }
    }

    /// Record `reads` as the files the failed load reached. Only a refusal keeps them.
    pub(crate) fn with_reads(mut self, reads: Vec<PathBuf>) -> Self {
        if let ConfigError::Rejected(rejected) = &mut self {
            rejected.reads = reads;
        }
        self
    }

    /// Read the config file at `path`, naming it `label` in errors, within
    /// [`crate::MAX_CONFIG_BYTES`].
    pub(crate) fn read_config(path: &std::path::Path, label: &str) -> Result<String, Self> {
        use crate::read::{MAX_CONFIG_BYTES, ReadFailure, read_text};
        read_text(path, MAX_CONFIG_BYTES).map_err(|e| match e {
            ReadFailure::TooLarge(cap) => ConfigError::TooLarge {
                path: label.to_owned(),
                cap,
                file: Some(path.to_path_buf()),
            },
            ReadFailure::Io(source) => ConfigError::Io {
                path: label.to_owned(),
                source,
                file: Some(path.to_path_buf()),
            },
        })
    }

    /// The stable error class: the first diagnostic's code, or `config-io` for a read failure.
    #[must_use]
    pub fn code(&self) -> Code {
        match self {
            ConfigError::Io { .. } => Code::Io,
            ConfigError::TooLarge { .. } => Code::TooLarge,
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
            ConfigError::Io { .. } | ConfigError::TooLarge { .. } => {
                format!("error[{}]: {self}", self.code().as_str())
            }
            ConfigError::Rejected(r) => r.render(),
        }
    }
}
