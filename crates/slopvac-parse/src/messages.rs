//! The messages `slopvac-parse` reports, as one catalog that the binary registers through
//! [`message_entries`].

use slopvac_messages::{Catalog, Entry};

/// Why a document did not parse.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum ParseMessage {
    /// The body span does not fit the source. The text of the span error follows.
    #[code("parse-body-span-invalid")]
    #[text("invalid body span: {reason}")]
    BodySpan { reason: String },
    /// The MDX parser refused the body. The parser's message follows.
    #[code("parse-mdx-syntax")]
    #[text("MDX syntax error: {reason}")]
    MdxSyntax { reason: String },
    /// The document needs more work than the parse budget allows.
    #[code("parse-budget-exceeded")]
    #[text("the document needs more work than the parse budget allows")]
    Budget,
    /// The segments of an HTML stream do not ascend, or they overlap.
    #[code("parse-segments-disordered")]
    #[text("the HTML segments must ascend and not overlap")]
    Segments,
    /// The document is longer than the parser accepts.
    #[code("parse-document-too-large")]
    #[text("the document is longer than the parser accepts")]
    TooLarge,
}

/// The entries of the parse catalog.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    ParseMessage::entries()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use slopvac_messages::Kind;

    use super::*;

    #[rstest]
    #[case::body_span(
        ParseMessage::BodySpan { reason: "end 9 is past 4 bytes".to_owned() },
        "invalid body span: end 9 is past 4 bytes"
    )]
    #[case::mdx(
        ParseMessage::MdxSyntax { reason: "unexpected end".to_owned() },
        "MDX syntax error: unexpected end"
    )]
    #[case::budget(
        ParseMessage::Budget,
        "the document needs more work than the parse budget allows"
    )]
    #[case::segments(
        ParseMessage::Segments,
        "the HTML segments must ascend and not overlap"
    )]
    #[case::too_large(
        ParseMessage::TooLarge,
        "the document is longer than the parser accepts"
    )]
    fn each_message_renders_its_published_text(
        #[case] message: ParseMessage,
        #[case] expected: &str,
    ) {
        assert_eq!(message.to_string(), expected);
    }

    #[test]
    fn the_catalog_registers_every_code_as_text() {
        let entries: Vec<_> = message_entries().iter().map(|e| (e.code, e.kind)).collect();
        assert_eq!(
            entries,
            [
                ("parse-body-span-invalid", Kind::Text),
                ("parse-mdx-syntax", Kind::Text),
                ("parse-budget-exceeded", Kind::Text),
                ("parse-segments-disordered", Kind::Text),
                ("parse-document-too-large", Kind::Text),
            ]
        );
    }
}
