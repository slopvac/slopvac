//! The messages `slopvac-pos` reports, as one catalog that the binary registers through
//! [`message_entries`].

use slopvac_messages::{Catalog, Entry};

/// Why a tagger model file was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum PosMessage {
    /// The file does not start with the model magic.
    #[code("pos-model-bad-magic")]
    #[text("not a prose tagger model (bad magic)")]
    BadMagic,
    /// The file ends before a field it declares.
    #[code("pos-model-truncated")]
    #[text("model file is truncated")]
    Truncated,
    /// A field holds a value the format does not allow.
    #[code("pos-model-corrupt")]
    #[text("model file is corrupt: {reason}")]
    Corrupt { reason: String },
    /// The model uses a feature template that is not implemented. The template is shown
    /// quoted.
    #[code("pos-model-unsupported-template")]
    #[text("unsupported feature template {template}; only the rune template (1) is implemented")]
    UnsupportedTemplate { template: String },
    /// The model emits a tag set other than Penn Treebank. The tag set is shown quoted.
    #[code("pos-model-unsupported-tagset")]
    #[text("unsupported tag set {tagset}; only \"penn\" is implemented")]
    UnsupportedTagset { tagset: String },
}

/// The entries of the tagger model catalog.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    PosMessage::entries()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use slopvac_messages::Kind;

    use super::*;

    #[rstest]
    #[case::bad_magic(PosMessage::BadMagic, "not a prose tagger model (bad magic)")]
    #[case::truncated(PosMessage::Truncated, "model file is truncated")]
    #[case::corrupt(
        PosMessage::Corrupt { reason: "class count".to_owned() },
        "model file is corrupt: class count"
    )]
    #[case::template(
        PosMessage::UnsupportedTemplate { template: "\"2\"".to_owned() },
        "unsupported feature template \"2\"; only the rune template (1) is implemented"
    )]
    #[case::tagset(
        PosMessage::UnsupportedTagset { tagset: "\"ud\"".to_owned() },
        "unsupported tag set \"ud\"; only \"penn\" is implemented"
    )]
    fn each_message_renders_its_published_text(
        #[case] message: PosMessage,
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
                ("pos-model-bad-magic", Kind::Text),
                ("pos-model-truncated", Kind::Text),
                ("pos-model-corrupt", Kind::Text),
                ("pos-model-unsupported-template", Kind::Text),
                ("pos-model-unsupported-tagset", Kind::Text),
            ]
        );
    }
}
