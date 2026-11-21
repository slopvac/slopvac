//! The messages `slopvac-diagrams` reports: the sentences of the label and prose checks, and
//! the fixed reasons an extraction gives when it cannot read a diagram in full.
//!
//! [`DiagramMessage`] holds the sentences with parameters. [`DiagramPhrase`] holds the fixed
//! reasons, which [`crate::ExtractionStatus`] carries as `&'static str` through
//! [`DiagramPhrase::as_static`]. The binary registers both through [`message_entries`] and
//! [`phrase_entries`]. No message holds diagram text: the reasons are fixed and the checks
//! name only counts.

use slopvac_messages::{Catalog, Entry};

/// The sentence a signal hit reports.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum DiagramMessage {
    /// A note of six words or more.
    #[code("diagram-signal-note-long")]
    #[text("This note has {count} words. Name the component or cut the note to a few words.")]
    NoteLong {
        #[count]
        count: usize,
    },
    /// A label of seven words or more.
    #[code("diagram-signal-label-run-on")]
    #[text(
        "This label has {count} words. Use a short noun phrase and move the detail to the text."
    )]
    LabelRunOn {
        #[count]
        count: usize,
    },
    /// A label that holds a name and a description.
    #[code("diagram-signal-label-title-plus-gloss")]
    #[text(
        "This label holds a name and a description. Keep the name and move the description to the text."
    )]
    LabelTitlePlusGloss,
    /// Prose that points at the diagram.
    #[code("diagram-signal-meta-reference")]
    #[text(
        "This sentence points at the diagram. Say what the system does and let the diagram show the shape."
    )]
    MetaReference,
    /// Prose that walks through the labels of the diagram.
    #[code("diagram-signal-prose-restates-labels")]
    #[text(
        "The text names {mentioned} of the {labels} labels in the diagram. Keep what the diagram cannot show, such as reasons, limits and failure modes."
    )]
    ProseRestatesLabels {
        #[count]
        mentioned: usize,
        #[count]
        labels: usize,
    },
}

/// The fixed reason of a failed extraction. Each reads as a `&'static str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Catalog)]
pub(crate) enum DiagramPhrase {
    /// A Mermaid front matter block has no closing line.
    #[code("diagram-reason-front-matter-open")]
    #[text("front matter is not closed")]
    FrontMatterOpen,
    /// The Mermaid diagram type is not one the crate reads.
    #[code("diagram-reason-mermaid-unsupported")]
    #[text("unsupported Mermaid diagram type")]
    MermaidUnsupported,
    /// A fenced block range does not lie inside the source.
    #[code("diagram-reason-block-outside-source")]
    #[text("block range is outside the source")]
    BlockOutsideSource,
    /// A size or work limit stopped the read.
    #[code("diagram-reason-budget-reached")]
    #[text("a size or work limit was reached")]
    BudgetReached,
    /// A binary format was given as text.
    #[code("diagram-reason-binary-needs-bytes")]
    #[text("binary format needs byte input")]
    BinaryNeedsBytes,
    /// Binary input is not valid UTF-8.
    #[code("diagram-reason-not-utf8")]
    #[text("input is not valid UTF-8")]
    NotUtf8,
    /// A `PlantUML` block has no `@enduml`.
    #[code("diagram-reason-missing-enduml")]
    #[text("missing @enduml")]
    MissingEnduml,
    /// A `PlantUML` block ends with another terminator than `@enduml`.
    #[code("diagram-reason-mismatched-terminator")]
    #[text("mismatched terminator")]
    MismatchedTerminator,
    /// The `PlantUML` diagram type is not one the crate reads.
    #[code("diagram-reason-plantuml-unsupported")]
    #[text("unsupported PlantUML diagram type")]
    PlantUmlUnsupported,
    /// A draw.io file holds no diagram source.
    #[code("diagram-reason-no-diagram-source")]
    #[text("no diagram source")]
    NoDiagramSource,
    /// A draw.io page holds bad percent encoding.
    #[code("diagram-reason-invalid-percent-encoding")]
    #[text("invalid percent encoding")]
    InvalidPercentEncoding,
    /// A draw.io file is not well-formed XML.
    #[code("diagram-reason-invalid-xml")]
    #[text("invalid XML")]
    InvalidXml,
    /// A draw.io file has no diagram pages.
    #[code("diagram-reason-no-diagram-pages")]
    #[text("no diagram pages")]
    NoDiagramPages,
    /// A draw.io page has content the crate does not recognise.
    #[code("diagram-reason-unrecognised-page")]
    #[text("unrecognised page content")]
    UnrecognisedPage,
    /// A draw.io page is not a graph model.
    #[code("diagram-reason-not-graph-model")]
    #[text("page is not a graph model")]
    NotGraphModel,
    /// The file is not a draw.io document.
    #[code("diagram-reason-not-drawio")]
    #[text("not a draw.io document")]
    NotDrawio,
    /// A compressed draw.io page is not valid base64.
    #[code("diagram-reason-invalid-base64")]
    #[text("invalid base64 page")]
    InvalidBase64,
    /// A compressed draw.io page does not inflate.
    #[code("diagram-reason-invalid-compressed")]
    #[text("invalid compressed page")]
    InvalidCompressed,
    /// An Excalidraw scene has no `elements` array.
    #[code("diagram-reason-no-elements")]
    #[text("no elements array")]
    NoElements,
    /// An Excalidraw element is not an object.
    #[code("diagram-reason-element-not-object")]
    #[text("element is not an object")]
    ElementNotObject,
    /// An Excalidraw text element has no text.
    #[code("diagram-reason-text-element-empty")]
    #[text("text element has no text")]
    TextElementEmpty,
}

/// The entries of the sentences with parameters.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    DiagramMessage::entries()
}

/// The entries of the fixed reasons.
#[must_use]
pub fn phrase_entries() -> &'static [Entry] {
    DiagramPhrase::entries()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use slopvac_messages::Kind;

    use super::*;

    #[rstest]
    #[case::note_long(
        DiagramMessage::NoteLong { count: 8 },
        "This note has 8 words. Name the component or cut the note to a few words."
    )]
    #[case::label_run_on(
        DiagramMessage::LabelRunOn { count: 9 },
        "This label has 9 words. Use a short noun phrase and move the detail to the text."
    )]
    #[case::title_plus_gloss(
        DiagramMessage::LabelTitlePlusGloss,
        "This label holds a name and a description. Keep the name and move the description to the text."
    )]
    #[case::meta_reference(
        DiagramMessage::MetaReference,
        "This sentence points at the diagram. Say what the system does and let the diagram show the shape."
    )]
    #[case::restates_labels(
        DiagramMessage::ProseRestatesLabels { mentioned: 3, labels: 4 },
        "The text names 3 of the 4 labels in the diagram. Keep what the diagram cannot show, such as reasons, limits and failure modes."
    )]
    fn each_signal_sentence_renders_its_published_text(
        #[case] message: DiagramMessage,
        #[case] expected: &str,
    ) {
        assert_eq!(message.to_string(), expected);
    }

    #[rstest]
    #[case::front_matter(DiagramPhrase::FrontMatterOpen, "front matter is not closed")]
    #[case::mermaid(DiagramPhrase::MermaidUnsupported, "unsupported Mermaid diagram type")]
    #[case::outside(DiagramPhrase::BlockOutsideSource, "block range is outside the source")]
    #[case::budget(DiagramPhrase::BudgetReached, "a size or work limit was reached")]
    #[case::bytes(DiagramPhrase::BinaryNeedsBytes, "binary format needs byte input")]
    #[case::utf8(DiagramPhrase::NotUtf8, "input is not valid UTF-8")]
    #[case::enduml(DiagramPhrase::MissingEnduml, "missing @enduml")]
    #[case::terminator(DiagramPhrase::MismatchedTerminator, "mismatched terminator")]
    #[case::plantuml(
        DiagramPhrase::PlantUmlUnsupported,
        "unsupported PlantUML diagram type"
    )]
    #[case::source(DiagramPhrase::NoDiagramSource, "no diagram source")]
    #[case::percent(DiagramPhrase::InvalidPercentEncoding, "invalid percent encoding")]
    #[case::xml(DiagramPhrase::InvalidXml, "invalid XML")]
    #[case::pages(DiagramPhrase::NoDiagramPages, "no diagram pages")]
    #[case::unrecognised(DiagramPhrase::UnrecognisedPage, "unrecognised page content")]
    #[case::graph(DiagramPhrase::NotGraphModel, "page is not a graph model")]
    #[case::drawio(DiagramPhrase::NotDrawio, "not a draw.io document")]
    #[case::base64(DiagramPhrase::InvalidBase64, "invalid base64 page")]
    #[case::compressed(DiagramPhrase::InvalidCompressed, "invalid compressed page")]
    #[case::elements(DiagramPhrase::NoElements, "no elements array")]
    #[case::element(DiagramPhrase::ElementNotObject, "element is not an object")]
    #[case::text(DiagramPhrase::TextElementEmpty, "text element has no text")]
    fn each_reason_reads_as_its_published_text(
        #[case] phrase: DiagramPhrase,
        #[case] expected: &str,
    ) {
        assert_eq!(phrase.as_static(), expected);
    }

    #[test]
    fn the_catalogs_register_each_code_as_text() {
        let codes: Vec<_> = message_entries().iter().map(|e| (e.code, e.kind)).collect();
        assert_eq!(
            codes,
            [
                ("diagram-signal-note-long", Kind::Text),
                ("diagram-signal-label-run-on", Kind::Text),
                ("diagram-signal-label-title-plus-gloss", Kind::Text),
                ("diagram-signal-meta-reference", Kind::Text),
                ("diagram-signal-prose-restates-labels", Kind::Text),
            ]
        );
        let phrases = phrase_entries();
        assert_eq!(phrases.len(), 21);
        assert!(phrases.iter().all(|e| e.kind == Kind::Text));
        assert!(
            phrases
                .iter()
                .all(|e| e.code.starts_with("diagram-reason-"))
        );
    }
}
