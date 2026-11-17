//! Errors with stable codes.

use std::fmt;
use std::path::PathBuf;

use slopvac_config::ConfigError;
use slopvac_config::schema::{Shape, config_shape};
use slopvac_model::{KebabName, NonEmptyString, Notice, NoticeLevel};

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
    ///
    /// Display carries only the redacted summary. The snippets are behind
    /// [`AgentsError::diagnostics`].
    #[error("{source}")]
    Config {
        /// The `config-*` code.
        code: &'static str,
        /// The cause, with its values redacted.
        #[source]
        source: ConfigFailure,
    },
    /// `slopvac-text` failed to segment or tokenize.
    #[error("text segmentation failed: {0}")]
    Text(#[from] slopvac_text::TextError),
    /// An I/O failure.
    #[error("{path}: {source}")]
    Io {
        /// The file involved.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
}

/// A [`ConfigError`] whose `Display` and `Debug` never print config values.
///
/// The wrapped error renders source snippets, and a snippet of an invalid setting can hold
/// the vocabulary terms the user wrote. This wrapper shows the stable code, the key path and
/// `file:line:column` only, so an error chain is safe to log. Callers who may show the
/// snippets to the user ask for them with [`ConfigFailure::diagnostics`]. The wrapped
/// error is not a `source()`, because its own `Display` is not redacted.
pub struct ConfigFailure(ConfigError);

impl ConfigFailure {
    pub(crate) fn new(error: ConfigError) -> Self {
        Self(error)
    }

    /// The stable `config-*` code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.0.code().as_str()
    }

    /// Each path the diagnostics print, exactly as they print it, so a caller can replace
    /// it whole instead of guessing where a path ends.
    #[must_use]
    pub fn paths(&self) -> Vec<String> {
        match &self.0 {
            ConfigError::Io { path, .. } | ConfigError::TooLarge { path, .. } => vec![path.clone()],
            ConfigError::Rejected(_) => Vec::new(),
        }
    }

    /// The rendered diagnostics, with source snippets. May contain config values: show
    /// them to the user, never log them.
    #[must_use]
    pub fn diagnostics(&self) -> String {
        self.0.render()
    }
}

/// The key path as a JSON pointer that names only what the schema defines.
///
/// Unknown keys and data keys (such as rule names) come from the user's repository and may
/// be private terms, so each shows as `<key>`. A list index shows as the number. The full
/// path stays in [`ConfigFailure::diagnostics`].
fn redacted_pointer(path: &[String]) -> String {
    if path.is_empty() {
        return "(document)".to_owned();
    }
    let root = config_shape();
    let mut shape = Some(&root);
    let mut out = String::new();
    for segment in path {
        out.push('/');
        // `Either` is a table when the path goes on into a key.
        if let Some(Shape::Either { table, .. }) = shape {
            shape = Some(table);
        }
        match shape {
            Some(Shape::Table(t)) => {
                if let Some((name, next)) = t.fields.iter().find(|(name, _)| name == segment) {
                    out.push_str(name);
                    shape = Some(next);
                } else {
                    out.push_str("<key>");
                    shape = None;
                }
            }
            Some(Shape::Map { value, .. }) => {
                out.push_str("<key>");
                shape = Some(value);
            }
            Some(Shape::List { item, .. }) if segment.bytes().all(|b| b.is_ascii_digit()) => {
                out.push_str(segment);
                shape = Some(item);
            }
            _ => {
                out.push_str("<key>");
                shape = None;
            }
        }
    }
    out
}

impl fmt::Display for ConfigFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            // The path is the one the caller gave and the reason is a fixed phrase.
            ConfigError::Io { .. } | ConfigError::TooLarge { .. } => {
                write!(f, "error[{}]: {}", self.code(), self.0)
            }
            ConfigError::Rejected(r) => {
                for (i, d) in r.diagnostics.iter().enumerate() {
                    if i > 0 {
                        f.write_str("\n")?;
                    }
                    let (line, col) = d.location(&r.text);
                    let key = redacted_pointer(&d.path);
                    write!(
                        f,
                        "error[{}]: invalid setting {key} at {}:{line}:{col}",
                        d.code.as_str(),
                        r.file
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Debug for ConfigFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ConfigFailure")
            .field(&format_args!("{self}"))
            .finish()
    }
}

impl std::error::Error for ConfigFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            // The operating-system error carries no config value.
            ConfigError::Io { source, .. } => Some(source),
            ConfigError::TooLarge { .. } | ConfigError::Rejected(_) => None,
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
            Self::Text(_) => "claims-text-failed",
            Self::Io { .. } => "setup-io",
        }
    }

    /// The config diagnostics with source snippets, when this is a config error. They may hold
    /// config values: show them to the user, never log them.
    #[must_use]
    pub fn diagnostics(&self) -> Option<String> {
        match self {
            Self::Config { source, .. } => Some(source.diagnostics()),
            _ => None,
        }
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Build a [`Notice`] for the workflows in this crate.
///
/// # Errors
/// The code is not kebab-case or the message is empty. Both are coding mistakes.
pub(crate) fn notice(
    code: &'static str,
    level: NoticeLevel,
    message: String,
) -> Result<Notice, AgentsError> {
    let invalid = |what: String| AgentsError::Coded {
        code: "agents-notice-invalid",
        message: format!("cannot build notice {code}: {what}"),
    };
    Ok(Notice {
        code: KebabName::new(code).map_err(|e| invalid(e.to_string()))?,
        level,
        message: NonEmptyString::new(message).map_err(|e| invalid(e.to_string()))?,
        path: None,
        data: serde_json::Map::new(),
    })
}
