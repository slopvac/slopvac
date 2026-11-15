//! Tags, the tagger trait, the registry and the context gate.

use std::collections::BTreeSet;
use std::ops::Range;

use crate::{DocumentView, Error};

/// A context tag over a byte span.
#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    /// Context tag kind, such as `change-note`.
    pub kind: String,
    /// Byte span in the file.
    pub span: Range<usize>,
    /// Confidence in 0..=1.
    pub confidence: f32,
    /// Why the tagger fired.
    pub evidence: String,
}

/// A deterministic tagger.
pub trait Tagger: Send + Sync {
    /// Stable tagger id.
    fn id(&self) -> &str;
    /// Tags for one document.
    fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error>;
}

/// Ordered set of taggers.
#[derive(Default)]
pub struct TagRegistry {
    taggers: Vec<Box<dyn Tagger>>,
}

impl TagRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a tagger. Ids are unique.
    pub fn register(&mut self, tagger: Box<dyn Tagger>) -> Result<(), Error> {
        if self.taggers.iter().any(|t| t.id() == tagger.id()) {
            return Err(Error::DuplicateTagger(tagger.id().to_string()));
        }
        self.taggers.push(tagger);
        Ok(())
    }

    /// Registered tagger ids in order.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.taggers.iter().map(|t| t.id()).collect()
    }

    /// Run every tagger and index the result. The first tagger error stops the run.
    pub fn tag_document(&self, doc: &dyn DocumentView) -> Result<TagIndex, Error> {
        let mut tags = Vec::new();
        for t in &self.taggers {
            tags.extend(t.tag(doc)?);
        }
        Ok(TagIndex::from_tags(tags))
    }
}

/// Tags of one document, sorted by span.
#[derive(Debug, Default, Clone)]
pub struct TagIndex {
    tags: Vec<Tag>,
}

impl TagIndex {
    /// Index `tags`: sorted by span, with duplicates removed.
    #[must_use]
    pub fn from_tags(mut tags: Vec<Tag>) -> Self {
        tags.sort_by(|a, b| {
            (a.span.start, a.span.end, &a.kind).cmp(&(b.span.start, b.span.end, &b.kind))
        });
        tags.dedup_by(|a, b| a.kind == b.kind && a.span == b.span);
        Self { tags }
    }

    /// All tags in span order.
    #[must_use]
    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }

    /// Kinds of the tags whose span contains `span`, sorted and unique.
    #[must_use]
    pub fn kinds_at(&self, span: &Range<usize>) -> Vec<&str> {
        let set: BTreeSet<&str> = self
            .tags
            .iter()
            .filter(|t| t.span.start <= span.start && span.end <= t.span.end)
            .map(|t| t.kind.as_str())
            .collect();
        set.into_iter().collect()
    }

    /// Kinds present anywhere in the document, sorted and unique.
    #[must_use]
    pub fn kinds_seen(&self) -> Vec<&str> {
        let set: BTreeSet<&str> = self.tags.iter().map(|t| t.kind.as_str()).collect();
        set.into_iter().collect()
    }
}

/// A rule's `contexts` field.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextGate {
    /// The rule runs only where one of these tags is present.
    #[serde(default)]
    pub only: Vec<String>,
    /// The rule is silent where any of these tags is present.
    #[serde(default)]
    pub skip: Vec<String>,
}

impl ContextGate {
    /// Whether a rule with this gate may report on text that carries `kinds`.
    #[must_use]
    pub fn allows(&self, kinds: &[&str]) -> bool {
        if self.skip.iter().any(|s| kinds.contains(&s.as_str())) {
            return false;
        }
        self.only.is_empty() || self.only.iter().any(|o| kinds.contains(&o.as_str()))
    }
}
