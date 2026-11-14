//! A paragraph that ends with a colon and names a version leads a list of changes.

use crate::{BlockKind, DocumentView, Error, Pattern, Tag, Tagger};

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

    fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
        let blocks = doc.blocks();
        let mut out = Vec::new();
        let mut leading = false;
        for b in &blocks {
            match b.kind {
                BlockKind::ListItem if leading => out.push(Tag {
                    kind: "change-note".to_string(),
                    span: b.span.clone(),
                    confidence: 0.6,
                    evidence: "change-note-list-lead-in: list under a version lead-in".to_string(),
                }),
                BlockKind::ListItem => {}
                BlockKind::Paragraph => {
                    let t = b.text.trim_end();
                    leading = t.ends_with(':') && self.version.is_match(t)?;
                }
                _ => leading = false,
            }
        }
        Ok(out)
    }
}
