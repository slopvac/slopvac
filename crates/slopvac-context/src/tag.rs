//! Tags, the tagger trait, the registry and the context gate.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::{DocumentView, Error, work};

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
    /// Per kind, over every tag: answers [`Self::kinds_at`].
    all: Vec<KindRuns>,
    /// Per kind, over the tags of a single piece: answers the single-piece part of
    /// [`Self::kinds_covering`].
    single: Vec<KindRuns>,
    /// The tags of several pieces, for the part of [`Self::kinds_covering`] that needs them.
    multi: Stab,
}

/// The tags of one kind in start order, with the greatest end among the tags up to each one.
/// A span lies inside a tag of this kind when the greatest end among the tags that start at
/// or before the span's start reaches the span's end.
#[derive(Debug, Clone)]
struct KindRuns {
    kind: String,
    starts: Vec<usize>,
    max_end: Vec<usize>,
}

impl KindRuns {
    /// One entry for each kind among `tags`, sorted by kind. `tags` are in start order.
    fn build<'t>(tags: impl Iterator<Item = &'t Tag>) -> Vec<Self> {
        let mut by_kind: BTreeMap<&str, Self> = BTreeMap::new();
        for t in tags {
            let runs = by_kind.entry(t.kind.as_str()).or_insert_with(|| Self {
                kind: t.kind.clone(),
                starts: Vec::new(),
                max_end: Vec::new(),
            });
            let end = runs
                .max_end
                .last()
                .map_or(t.span.end, |&m| m.max(t.span.end));
            runs.starts.push(t.span.start);
            runs.max_end.push(end);
        }
        by_kind.into_values().collect()
    }

    /// Kinds with a tag that contains `start..end`, sorted and unique. The work is one binary
    /// search for each kind.
    fn kinds_containing(runs: &[Self], start: usize, end: usize) -> Vec<&str> {
        let mut kinds = Vec::new();
        for r in runs {
            work::lookup(search_steps(r.starts.len()));
            let upto = r.starts.partition_point(|&s| s <= start);
            if upto > 0 && r.max_end[upto - 1] >= end {
                kinds.push(r.kind.as_str());
            }
        }
        kinds
    }
}

/// Probes of a binary search over `len` entries.
fn search_steps(len: usize) -> usize {
    1 + len.checked_ilog2().map_or(0, |l| l as usize)
}

/// A segment tree over tags in start order that holds the greatest end of each node, so a
/// query visits only the tags that contain a span, plus the path to each.
#[derive(Debug, Default, Clone)]
struct Stab {
    /// Indexes into the tags, in start order.
    ids: Vec<usize>,
    starts: Vec<usize>,
    /// Leaves from `leaves`, then the nodes above, as in a heap.
    tree: Vec<usize>,
    leaves: usize,
}

impl Stab {
    fn build(tags: &[Tag], ids: Vec<usize>) -> Self {
        let leaves = ids.len().next_power_of_two();
        let mut tree = vec![0; 2 * leaves];
        for (j, &i) in ids.iter().enumerate() {
            tree[leaves + j] = tags[i].span.end;
        }
        for n in (1..leaves).rev() {
            tree[n] = tree[2 * n].max(tree[2 * n + 1]);
        }
        let starts = ids.iter().map(|&i| tags[i].span.start).collect();
        Self {
            ids,
            starts,
            tree,
            leaves,
        }
    }

    /// Call `f` with the index of each tag that contains `start..end`.
    fn containing(&self, start: usize, end: usize, f: &mut impl FnMut(usize)) {
        work::lookup(search_steps(self.starts.len()));
        let upto = self.starts.partition_point(|&s| s <= start);
        if upto > 0 {
            self.visit(1, 0, self.leaves, upto, end, f);
        }
    }

    fn visit(
        &self,
        node: usize,
        lo: usize,
        hi: usize,
        upto: usize,
        end: usize,
        f: &mut impl FnMut(usize),
    ) {
        if lo >= upto || self.tree[node] < end {
            return;
        }
        work::lookup(1);
        if hi - lo == 1 {
            f(self.ids[lo]);
            return;
        }
        let mid = lo + (hi - lo) / 2;
        self.visit(2 * node, lo, mid, upto, end, f);
        self.visit(2 * node + 1, mid, hi, upto, end, f);
    }
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
        let all = KindRuns::build(tags.iter());
        let single = KindRuns::build(
            tags.iter()
                .enumerate()
                .filter(|(i, _)| !members.contains_key(&groups[*i]))
                .map(|(_, t)| t),
        );
        let multi_ids = (0..tags.len())
            .filter(|&i| members.contains_key(&groups[i]))
            .collect();
        let multi = if members.is_empty() {
            Stab::default()
        } else {
            Stab::build(&tags, multi_ids)
        };
        Self {
            tags,
            groups,
            members,
            all,
            single,
            multi,
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
        // A tag of one piece covers every range when it covers the least start to the
        // greatest end.
        let from = ranges.iter().map(|r| r.start).min().unwrap_or(first.start);
        let to = ranges.iter().map(|r| r.end).max().unwrap_or(first.end);
        let mut set: BTreeSet<&str> = KindRuns::kinds_containing(&self.single, from, to)
            .into_iter()
            .collect();
        if !self.multi.ids.is_empty() {
            self.multi.containing(first.start, first.end, &mut |i| {
                let kind = self.tags[i].kind.as_str();
                if set.contains(kind) {
                    return;
                }
                let pieces = self
                    .members
                    .get(&self.groups[i])
                    .map_or(&[][..], Vec::as_slice);
                if ranges
                    .iter()
                    .all(|r| pieces.iter().any(|&m| covers(&self.tags[m], r)))
                {
                    set.insert(kind);
                }
            });
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
        KindRuns::kinds_containing(&self.all, span.start, span.end)
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
