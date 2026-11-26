//! A paragraph that ends with a colon and names a version leads a list of changes.

use crate::{BlockKind, DocumentView, Error, Pattern, Tag, TagSink, Tagger};

const VERSION: &str = r"\bv\d+(?:\.\d+)*\b|\b\d+\.\d+(?:\.\d+)?\b";

/// Tags the list items under a version lead-in such as `In 2.0:`.
pub struct ListLeadInTagger {
    version: Pattern,
}

impl ListLeadInTagger {
    /// Compile the pattern.
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            version: Pattern::new_caseless(VERSION)?,
        })
    }
}

impl Tagger for ListLeadInTagger {
    fn id(&self) -> &'static str {
        "change-note-list-lead-in"
    }

    fn emitted_tags(&self) -> Vec<&str> {
        vec!["change-note"]
    }

    fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
        let mut sink = TagSink::unbounded();
        self.tag_into(doc, &mut sink)?;
        Ok(sink.into_tags())
    }

    fn tag_into(&self, doc: &dyn DocumentView, sink: &mut TagSink) -> Result<(), Error> {
        let blocks = doc.blocks();
        let mut leading = false;
        for b in &blocks {
            match b.kind {
                BlockKind::ListItem if leading => sink.push_with(|| Tag {
                    kind: "change-note".to_string(),
                    span: b.span.clone(),
                    confidence: 0.6,
                    evidence: "change-note-list-lead-in: list under a version lead-in".into(),
                })?,
                BlockKind::ListItem => {}
                BlockKind::Paragraph => {
                    let t = b.text.trim_end();
                    leading = t.ends_with(':') && self.version.is_match(t)?;
                }
                _ => leading = false,
            }
        }
        Ok(())
    }
}
