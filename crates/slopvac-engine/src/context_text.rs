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
        Self::Context(raw_text(error))
    }
}

impl EngineError {
    /// The message of this error with its untrusted parts left raw, for a caller that
    /// escapes the text itself, once, where it writes it (a terminal line, a report).
    /// `Display` is the same text, escaped.
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

/// The wording of `ContextError`'s `Display`, with the untrusted parts unescaped.
///
/// A variant that has no wording here falls back to its `Display`, which is already escaped,
/// so a backslash in it is escaped twice. The conformance test below fails when the wording
/// here drifts from the `Display` of a variant it covers.
fn raw_text(error: &ContextError) -> String {
    match error {
        ContextError::Pattern { pattern, source } => {
            // ast-grep-ignore: user-text-literal
            format!("invalid pattern `{pattern}`: {source}")
        }
        ContextError::Match { pattern, source } => {
            // ast-grep-ignore: user-text-literal
            format!("pattern `{pattern}` failed while matching: {source}")
        }
        // ast-grep-ignore: user-text-literal
        ContextError::DuplicateTagger(id) => format!("tagger `{id}` is already registered"),
        // ast-grep-ignore: user-text-literal
        ContextError::UnknownGenre(id) => format!("unknown genre `{id}`"),
        ContextError::Glob { pattern, source } => format!("glob `{pattern}`: {source}"),
        ContextError::SignalGlob {
            genre,
            glob,
            source,
        } => format!("genre `{genre}`: glob `{glob}`: {source}"),
        ContextError::Read { path, source } => format!("{path}: {source}"),
        ContextError::TooLarge { path, cap } => {
            // ast-grep-ignore: user-text-literal
            format!("{path}: larger than the {cap}-byte document limit")
        }
        // ast-grep-ignore: user-text-literal
        ContextError::Unreadable { path } => format!("{path}: cannot read"),
        ContextError::Parse(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    /// A hostile id: a backslash, a newline and a bidi override.
    const HOSTILE: &str = "a\\b\n\u{202e}c";

    #[rstest]
    #[case::unknown_genre(ContextError::UnknownGenre(HOSTILE.into()))]
    #[case::duplicate_tagger(ContextError::DuplicateTagger(HOSTILE.into()))]
    #[case::unreadable(ContextError::Unreadable { path: HOSTILE.into() })]
    #[case::too_large(ContextError::TooLarge { path: HOSTILE.into(), cap: 7 })]
    #[case::parse(ContextError::Parse(HOSTILE.into()))]
    #[case::match_limit(ContextError::Match {
        pattern: HOSTILE.into(),
        source: slopvac_pcre2::MatchError::MatchLimit { limit: 1 },
    })]
    #[case::read(ContextError::Read {
        path: HOSTILE.into(),
        source: std::io::Error::other(HOSTILE),
    })]
    fn escaping_the_raw_text_once_gives_the_context_display(#[case] error: ContextError) {
        assert_eq!(slopvac_sanitize::line(&raw_text(&error)), error.to_string());
    }

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
