//! The messages `slopvac-text` reports, as one catalog that the binary registers through
//! [`message_entries`].

use slopvac_messages::{Catalog, Entry};

/// The failures of pattern compiling and matching.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum TextMessage {
    /// A pattern failed at match time, for example by reaching a PCRE2 limit.
    #[code("text-match-failed")]
    #[text("pattern match failed: {reason}")]
    MatchFailed { reason: String },
    /// A pattern did not compile.
    #[code("text-pattern-invalid")]
    #[text("pattern does not compile: {reason}")]
    PatternInvalid { reason: String },
}

/// The entries of the text catalog.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    TextMessage::entries()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::match_failed(
        TextMessage::MatchFailed { reason: "match limit of 7 exceeded".to_owned() },
        "pattern match failed: match limit of 7 exceeded"
    )]
    #[case::pattern_invalid(
        TextMessage::PatternInvalid { reason: "missing closing parenthesis".to_owned() },
        "pattern does not compile: missing closing parenthesis"
    )]
    fn each_message_renders_its_published_text(
        #[case] message: TextMessage,
        #[case] expected: &str,
    ) {
        assert_eq!(message.to_string(), expected);
    }

    #[test]
    fn the_catalog_registers_both_codes_as_text() {
        let entries: Vec<_> = message_entries().iter().map(|e| (e.code, e.kind)).collect();
        assert_eq!(
            entries,
            [
                ("text-match-failed", slopvac_messages::Kind::Text),
                ("text-pattern-invalid", slopvac_messages::Kind::Text),
            ]
        );
    }
}
