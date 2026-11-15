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
    /// The group of each tag. Tags of one group are the pieces of one tag.
    groups: Vec<usize>,
    /// For a group of several pieces, the indexes of its tags.
    members: std::collections::HashMap<usize, Vec<usize>>,
}

impl TagIndex {
    /// Index `tags`: sorted by span, with duplicates removed. Equal tags are not told apart.
    #[must_use]
    pub fn from_tags(mut tags: Vec<Tag>) -> Self {
        // Equal tags are one tag, whoever produced them.
        tags.sort_by(|a, b| {
            (a.span.start, a.span.end, &a.kind).cmp(&(b.span.start, b.span.end, &b.kind))
        });
        tags.dedup_by(|a, b| a.kind == b.kind && a.span == b.span);
        Self::from_pieces(tags.into_iter().enumerate().map(|(i, t)| (t, i)).collect())
    }

    /// Index tags that may be pieces of one tag: pieces with the same group number belong
    /// together. A tag on text that reads in another order than the source, such as a wrapped
    /// table cell, is split into the ranges of its own text, and a match that spans several
    /// of them is covered by the tag as a whole ([`Self::kinds_covering`]).
    #[must_use]
    pub fn from_pieces(mut pieces: Vec<(Tag, usize)>) -> Self {
        pieces.sort_by(|(a, ga), (b, gb)| {
            (a.span.start, a.span.end, &a.kind, ga).cmp(&(b.span.start, b.span.end, &b.kind, gb))
        });
        // Equal pieces of one group are one piece. Pieces of different groups stay, so every
        // group keeps all its members.
        pieces.dedup_by(|(a, ga), (b, gb)| a.kind == b.kind && a.span == b.span && ga == gb);
        let (tags, groups): (Vec<Tag>, Vec<usize>) = pieces.into_iter().unzip();
        let mut members: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, &g) in groups.iter().enumerate() {
            members.entry(g).or_default().push(i);
        }
        members.retain(|_, m| m.len() > 1);
        Self {
            tags,
            groups,
            members,
        }
    }

    /// Kinds of the tags that cover every one of `ranges`, sorted and unique. A tag covers a
    /// range when one of its pieces contains it, so ranges on different lines of one tag
    /// are covered by it. For one range this is [`Self::kinds_at`].
    #[must_use]
    pub fn kinds_covering(&self, ranges: &[Range<usize>]) -> Vec<&str> {
        let Some(first) = ranges.first() else {
            return Vec::new();
        };
        let covers = |t: &Tag, r: &Range<usize>| t.span.start <= r.start && r.end <= t.span.end;
        let mut set = BTreeSet::new();
        for (i, tag) in self.tags.iter().enumerate() {
            if !covers(tag, first) {
                continue;
            }
            let own = [i];
            let pieces = self
                .members
                .get(&self.groups[i])
                .map_or(&own[..], Vec::as_slice);
            if ranges
                .iter()
                .all(|r| pieces.iter().any(|&m| covers(&self.tags[m], r)))
            {
                set.insert(tag.kind.as_str());
            }
        }
        set.into_iter().collect()
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
