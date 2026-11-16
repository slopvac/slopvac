//! Errors with stable codes.

use std::path::PathBuf;

/// A setup or steering failure. Each variant has a stable kebab-case code.
#[derive(Debug, thiserror::Error)]
pub enum AgentsError {
    /// The target escapes the project or is not a Markdown file.
    #[error("unsafe steering target: {0}")]
    UnsafeTarget(String),
    /// The begin/end markers are missing one half or repeated.
    #[error("malformed or ambiguous slopvac markers in {0}; repair the begin/end pair first")]
    MalformedMarkers(PathBuf),
    /// The file is not UTF-8.
    #[error("steering must be UTF-8: {0}")]
    NotUtf8(PathBuf),
    /// The file changed between read and write.
    #[error("file changed during setup; retry: {0}")]
    Conflict(PathBuf),
    /// The path holds a file slopvac did not write.
    #[error("{0} is not a slopvac-managed file; move it aside to install it")]
    NotManaged(PathBuf),
    /// A usage mistake.
    #[error("{0}")]
    Usage(String),
    /// A review policy refusal with its own stable code.
    #[error("{message}")]
    Coded {
        /// The stable code.
        code: &'static str,
        /// The message.
        message: String,
    },
    /// The config could not be loaded; the code is the config crate's.
    #[error("{message}")]
    Config {
        /// The `config-*` code.
        code: &'static str,
        /// The rendered diagnostics.
        message: String,
    },
    /// An I/O failure.
    #[error("{path}: {source}")]
    Io {
        /// The file involved.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
}

impl From<slopvac_text::TextError> for AgentsError {
    fn from(e: slopvac_text::TextError) -> Self {
        Self::Coded {
            code: "claims-text-failed",
            message: format!("text segmentation failed: {e}"),
        }
    }
}

impl AgentsError {
    /// The stable error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsafeTarget(_) => "steering-unsafe-target",
            Self::MalformedMarkers(_) => "steering-malformed",
            Self::NotUtf8(_) => "steering-not-utf8",
            Self::Conflict(_) => "steering-conflict",
            Self::NotManaged(_) => "steering-not-managed",
            Self::Usage(_) => "setup-usage",
            Self::Coded { code, .. } | Self::Config { code, .. } => code,
            Self::Io { .. } => "setup-io",
        }
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
