//! Errors with stable codes.

use std::fmt;
use std::path::PathBuf;

use slopvac_config::ConfigError;
use slopvac_config::schema::{Shape, config_shape};
use slopvac_messages::Catalog;
use slopvac_model::{KebabName, NonEmptyString, Notice, NoticeLevel};

use crate::messages::{self, AgentsMessage, AgentsText, codes};

/// The registered code for a text that passes a budget. The `slopvac` binary reports the
/// engine's budget errors under the same code.
// TODO(8l4.21.6): take this code from the message registry. `slopvac_context::Error::code()`
// answers `context-tag-limit`, and the binary writes `context-budget-exceeded` as a literal in
// `engine.rs`, so no exported constant carries it yet.
const BUDGET_EXCEEDED: &str = "context-budget-exceeded";

/// A setup or steering failure. Each variant has a stable kebab-case code.
///
/// Every path a variant carries is relative to the project root, or to the working
/// directory for a file the user named on the command line. None is joined to the root, so a
/// caller shows `root.join(path)` and never has to guess whether the root is already in it.
#[derive(Debug, thiserror::Error)]
pub enum AgentsError {
    /// The target escapes the project or is not a Markdown file.
    #[error("{}", messages::unsafe_target(.0))]
    UnsafeTarget(String),
    /// The begin/end markers are missing one half or repeated.
    #[error("{}", messages::markers_malformed(.0))]
    MalformedMarkers(PathBuf),
    /// The file has an unterminated code fence that would hide the block.
    #[error("{}", messages::fence_unclosed(.0))]
    UnclosedFence(PathBuf),
    /// The file is not UTF-8.
    #[error("{}", messages::not_utf8(.0))]
    NotUtf8(PathBuf),
    /// The file changed between read and write.
    #[error("{}", messages::conflict(.0))]
    Conflict(PathBuf),
    /// The path holds a file slopvac did not write.
    #[error("{}", messages::not_managed(.0))]
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
    #[error("{}", messages::text_failed(.0))]
    Text(#[from] slopvac_text::TextError),
    /// A context budget was passed: the text holds more sentences than its size allows.
    #[error(transparent)]
    Context(#[from] slopvac_context::Error),
    /// An input passed a context budget. `shown` is its path as the user gave it, under the
    /// working directory, or `<external>/<file name>` outside it.
    #[error("{}: {source}", slopvac_sanitize::line(.shown))]
    InputBudget {
        /// The input, as shown to the user.
        shown: String,
        /// The budget refusal. Boxed, so the variant stays small.
        #[source]
        source: Box<slopvac_context::Error>,
    },
    /// A step failed after earlier files were already written.
    #[error("{}", messages::partial(.source, .changed))]
    Partial {
        /// Display paths of the files written before the failure.
        changed: Vec<String>,
        /// The failure.
        #[source]
        source: Box<AgentsError>,
    },
    /// An I/O failure.
    #[error("{}", messages::io_failure(.path, .source))]
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
            ConfigError::Io { .. } | ConfigError::TooLarge { .. } => f.write_str(
                &AgentsText::ConfigFailureLine {
                    code: self.code().to_owned(),
                    message: self.0.to_string(),
                }
                .to_string(),
            ),
            ConfigError::Rejected(r) => {
                for (i, d) in r.diagnostics.iter().enumerate() {
                    if i > 0 {
                        f.write_str("\n")?;
                    }
                    let (line, col) = d.location(&r.text);
                    let key = redacted_pointer(&d.path);
                    f.write_str(
                        &AgentsText::ConfigInvalidSetting {
                            code: d.code.as_str().to_owned(),
                            key,
                            file: r.file.clone(),
                            line,
                            column: col,
                        }
                        .to_string(),
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
            Self::UnsafeTarget(_) => codes::UNSAFE_TARGET,
            Self::MalformedMarkers(_) | Self::UnclosedFence(_) => codes::MALFORMED,
            Self::NotUtf8(_) => codes::NOT_UTF8,
            Self::Conflict(_) => codes::CONFLICT,
            Self::NotManaged(_) => codes::NOT_MANAGED,
            Self::Usage(_) => codes::USAGE,
            Self::Coded { code, .. } | Self::Config { code, .. } => code,
            Self::Partial { source, .. } => source.code(),
            Self::Text(_) => codes::TEXT_FAILED,
            Self::Context(_) | Self::InputBudget { .. } => BUDGET_EXCEEDED,
            Self::Io { .. } => codes::IO,
        }
    }

    /// The budget this error reports as passed, with its limit, when it is a context budget
    /// refusal: `("tags", n)` or `("sentences", n)`.
    #[must_use]
    pub fn budget(&self) -> Option<(&'static str, usize)> {
        match self {
            Self::Context(error) => error.budget(),
            Self::InputBudget { source, .. } => source.budget(),
            Self::Partial { source, .. } => source.budget(),
            _ => None,
        }
    }

    /// The input a budget refusal names, as shown to the user.
    #[must_use]
    pub fn input_path(&self) -> Option<&str> {
        match self {
            Self::InputBudget { shown, .. } => Some(shown),
            Self::Partial { source, .. } => source.input_path(),
            _ => None,
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

    /// A refusal with the stable code and the rendered text of `message`.
    pub(crate) fn coded(message: &AgentsMessage) -> Self {
        Self::Coded {
            code: message.code(),
            message: message.to_string(),
        }
    }

    /// A usage failure with the text `message`.
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self::Usage(
            AgentsMessage::Usage {
                message: message.into(),
            }
            .to_string(),
        )
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Build a [`Notice`] from a notice message of the catalog.
///
/// # Errors
/// The code is not kebab-case or the message is empty. Both are coding mistakes.
pub(crate) fn notice(level: NoticeLevel, message: &AgentsMessage) -> Result<Notice, AgentsError> {
    let code = message.code();
    let invalid = |what: String| {
        AgentsError::coded(&AgentsMessage::NoticeInvalid {
            notice_code: code.to_owned(),
            reason: what,
        })
    };
    Ok(Notice {
        code: KebabName::new(code).map_err(|e| invalid(e.to_string()))?,
        level,
        message: NonEmptyString::new(message.to_string()).map_err(|e| invalid(e.to_string()))?,
        path: None,
        data: serde_json::Map::new(),
    })
}
