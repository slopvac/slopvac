//! The text of a context failure that reaches `EngineError::Context`.
//!
//! `slopvac_context::Error` escapes the untrusted parts of its own `Display`. The engine error
//! must not carry that escaped text as its payload: its `Display` escapes the payload again,
//! and a caller that builds a report notice from the payload would escape it a third time.
//! The payload is the raw text, so each output escapes it exactly once.

use slopvac_context::Error as ContextError;

use crate::EngineError;
use crate::tagging::TagError;

impl EngineError {
    /// A context failure with its untrusted parts (patterns, ids, paths, messages) left raw.
    pub(crate) fn from_context(error: &ContextError) -> Self {
        Self::Context(error.raw_message())
    }
}

impl EngineError {
    /// The message of this error with its untrusted parts left unescaped. `Display` is the
    /// same text, escaped.
    ///
    /// The caller must escape the result once where it writes it (a terminal line or a
    /// report). It must never reach a log, a panic or an error chain.
    #[must_use]
    pub fn raw_message(&self) -> String {
        match self {
            Self::Context(raw) => crate::messages::context(raw).to_string(),
            other => other.to_string(),
        }
    }

    /// The engine error for a tagging failure.
    pub(crate) fn from_tag(error: TagError) -> Self {
        match error {
            TagError::Budget { limit } => Self::TagBuildBudget { limit },
            TagError::Tagger(error) => Self::from_context(&error),
            TagError::Unmapped => Self::Context(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hostile id: a backslash, a newline and a bidi override.
    const HOSTILE: &str = "a\\b\n\u{202e}c";

    #[test]
    fn the_raw_message_is_the_display_before_escaping() {
        let error = EngineError::from_context(&ContextError::UnknownGenre(HOSTILE.into()));
        assert_eq!(
            slopvac_sanitize::line(&error.raw_message()),
            error.to_string()
        );
    }

    #[test]
    fn the_engine_error_keeps_the_raw_text_and_displays_it_escaped_once() {
        let error = EngineError::from_context(&ContextError::UnknownGenre(HOSTILE.into()));
        let EngineError::Context(raw) = &error else {
            panic!("a context failure keeps its own variant: {error:?}");
        };
        assert_eq!(raw, &format!("unknown genre `{HOSTILE}`"));
        assert_eq!(
            error.to_string(),
            "context tagger failed: unknown genre `a\\\\b\\u{a}\\u{202e}c`"
        );
    }
}
