//! Tags, the tagger trait, the registry and the context gate.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

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
    /// Why the tagger fired. Pieces that a tag is split into share one allocation.
    pub evidence: Arc<str>,
}

/// A deterministic tagger.
pub trait Tagger: Send + Sync {
    /// Stable tagger id.
    fn id(&self) -> &str;
    /// Tags for one document.
    fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error>;
    /// Tags for one document, pushed into `sink`, which charges its ceiling before each tag
    /// is built. A tagger that can produce many tags overrides this, so that it stops at the
    /// ceiling without holding the tags past it. The default runs [`Self::tag`], which holds
    /// its whole result before the sink sees it.
    fn tag_into(&self, doc: &dyn DocumentView, sink: &mut TagSink) -> Result<(), Error> {
        for tag in self.tag(doc)? {
            sink.push_with(|| tag)?;
        }
        Ok(())
    }
}

/// A bounded collection of tags. It charges one tag against its ceiling before the tag is
/// built, so a producer that stops at the first error never holds more tags than the ceiling.
#[derive(Debug)]
pub struct TagSink {
    tags: Vec<Tag>,
    limit: usize,
    /// Address of the evidence of the last tag pushed, so that the work counter can tell a
    /// piece that shares it from a tag that holds text of its own.
    last_evidence: usize,
}

impl TagSink {
    /// A sink that takes at most `limit` tags.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            tags: Vec::new(),
            limit,
            last_evidence: 0,
        }
    }

    /// A sink with no ceiling.
    #[must_use]
    pub fn unbounded() -> Self {
        Self::new(usize::MAX)
    }

    /// Charge one tag, then build it with `make` and keep it. Fails with [`Error::TagLimit`],
    /// without calling `make`, when the sink already holds `limit` tags.
    pub fn push_with(&mut self, make: impl FnOnce() -> Tag) -> Result<(), Error> {
        if self.tags.len() >= self.limit {
            return Err(Error::TagLimit { limit: self.limit });
        }
        work::materialised(1);
        let tag = make();
        let address = tag.evidence.as_ptr().addr();
        if address != self.last_evidence {
            work::evidence(tag.evidence.len());
            self.last_evidence = address;
        }
        self.tags.push(tag);
        Ok(())
    }

    /// The tags kept so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// Whether the sink holds no tag.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// The tags, in the order they were pushed.
    #[must_use]
    pub fn into_tags(self) -> Vec<Tag> {
        self.tags
    }
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
        Ok(TagIndex::from_tags(self.collect_tags(doc, usize::MAX)?))
    }

    /// Run every tagger and return their tags in span order, with duplicates removed, before
    /// anything is indexed. The first tagger error stops the run. The run stops with
    /// [`Error::TagLimit`] once the taggers so far hold more than `max_tags` tags, so a
    /// document cannot make the caller build an index of unbounded size.
    pub fn collect_tags(&self, doc: &dyn DocumentView, max_tags: usize) -> Result<Vec<Tag>, Error> {
        self.collect_tags_with(doc, max_tags, &[])
    }

    /// Like [`Self::collect_tags`], with `extra` taggers run after the registered ones. Their
    /// emissions count against `max_tags` the same way, before duplicates go.
    pub fn collect_tags_with(
        &self,
        doc: &dyn DocumentView,
        max_tags: usize,
        extra: &[&dyn Tagger],
    ) -> Result<Vec<Tag>, Error> {
        let mut sink = TagSink::new(max_tags);
        for t in &self.taggers {
            t.tag_into(doc, &mut sink)?;
        }
        for t in extra {
            t.tag_into(doc, &mut sink)?;
        }
        let mut tags = sink.into_tags();
        normalize_tags(&mut tags);
        Ok(tags)
    }
}

/// Sort `tags` by span and kind, and drop duplicates: equal tags are one tag, whoever
/// produced them.
pub fn normalize_tags(tags: &mut Vec<Tag>) {
    tags.sort_by(|a, b| {
        (a.span.start, a.span.end, &a.kind).cmp(&(b.span.start, b.span.end, &b.kind))
    });
    tags.dedup_by(|a, b| a.kind == b.kind && a.span == b.span);
}

/// Tags of one document, sorted by span.
#[derive(Debug, Default, Clone)]
pub struct TagIndex {
    tags: Vec<Tag>,
    /// The group of each tag. Tags of one group are the pieces of one tag.
    groups: Vec<usize>,
    /// For a group of several pieces, its pieces.
    members: HashMap<usize, Pieces>,
    /// The groups that repeat an earlier group of the same kind and pieces, left out of the
    /// stabbing structures because the earlier group answers for them.
    repeated: HashSet<usize>,
    /// Per kind, over every tag: answers [`Self::kinds_at`].
    all: Spatial,
    /// Per kind, over the tags of a single piece: answers the single-piece part of
    /// [`Self::kinds_covering`]. None when every tag is a single piece, which `all` then
    /// answers, so the index is built once.
    single: Option<Spatial>,
    /// The tags of several pieces, for the part of [`Self::kinds_covering`] that needs them.
    multi: Stab,
    /// The same structures kept apart per kind, built on the first lookup that names kinds.
    by_kind: OnceLock<HashMap<String, PerKind>>,
    /// Caps the candidates [`Self::kinds_covering`] visits.
    budget: Budget,
}

/// The structures of the tags of one kind, so that asking whether that kind holds costs
/// `O(log n)` however many other kinds the document has.
#[derive(Debug, Default, Clone)]
struct PerKind {
    all: Spatial,
    /// The tags of a single piece, when some tags of this kind have several.
    single: Option<Spatial>,
    /// The tags of several pieces.
    multi: Stab,
}

/// A cap on the tags that [`TagIndex::kinds_covering`] looks at, shared by all its lookups.
///
/// A lookup in a table whose cell wraps visits every tag of the cell that holds its first
/// range, and many tags of different kinds can sit on one cell, so the visits of many
/// lookups can add up to more than the document is worth. Once the visits pass the cap,
/// every lookup stops at once and answers nothing, and [`TagIndex::lookup_budget_exceeded`]
/// tells the caller to discard the run.
#[derive(Debug)]
struct Budget {
    limit: usize,
    spent: AtomicUsize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            limit: usize::MAX,
            spent: AtomicUsize::new(0),
        }
    }
}

impl Clone for Budget {
    fn clone(&self) -> Self {
        Self {
            limit: self.limit,
            spent: AtomicUsize::new(self.spent.load(Ordering::Relaxed)),
        }
    }
}

impl Budget {
    fn exceeded(&self) -> bool {
        self.spent.load(Ordering::Relaxed) > self.limit
    }

    /// Count one visit. False once the cap is passed.
    fn charge(&self) -> bool {
        self.spent.fetch_add(1, Ordering::Relaxed) < self.limit
    }
}

/// Every tag of a document, indexed so that a lookup costs `O(log n + k)` for the `k` kinds
/// it answers, however many kinds and tags the document has.
///
/// A tag is dropped when an earlier tag of its kind starts no later and ends no earlier, since
/// anything it holds, that tag holds too. The tags kept for a kind then end in the order they
/// start, so for a span the last of them that starts at or before it is the one to ask: if
/// any tag of the kind holds the span, that one does. Each kept tag therefore answers the
/// spans whose last start falls between its own start and the start of the next tag of its
/// kind. A segment tree over start order stores each tag at the nodes that cover that range,
/// longest first. A lookup reads the nodes from one leaf to the root and stops each at the
/// first tag that ends too soon, and every tag it reads before that is a kind in the answer,
/// once.
#[derive(Debug, Default, Clone)]
struct Spatial {
    /// Starts of the kept tags, in order.
    starts: Vec<usize>,
    /// The kind names, sorted.
    kinds: Vec<String>,
    /// A bottom-up segment tree over the kept tags: the `(end, kind)` of the tags stored at
    /// each node, with the greatest end first.
    nodes: Vec<Vec<(usize, usize)>>,
}

impl Spatial {
    /// Index `tags`, which are in start order.
    fn build<'t>(tags: impl Iterator<Item = &'t Tag>) -> Self {
        let tags: Vec<&Tag> = tags.collect();
        work::build(tags.len());
        let kinds: Vec<String> = tags
            .iter()
            .map(|t| t.kind.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(str::to_string)
            .collect();
        let rank: HashMap<&str, usize> = kinds
            .iter()
            .enumerate()
            .map(|(i, k)| (k.as_str(), i))
            .collect();
        // The kept tags as `(start, end, kind)`, and for each the index of the next of its kind.
        let mut kept: Vec<(usize, usize, usize)> = Vec::new();
        let mut next: Vec<usize> = Vec::new();
        let mut last: Vec<Option<usize>> = vec![None; kinds.len()];
        for t in tags {
            let k = rank[t.kind.as_str()];
            if last[k].is_some_and(|l| kept[l].1 >= t.span.end) {
                continue;
            }
            if let Some(l) = last[k] {
                next[l] = kept.len();
            }
            last[k] = Some(kept.len());
            kept.push((t.span.start, t.span.end, k));
            next.push(usize::MAX);
        }
        let n = kept.len();
        let mut nodes: Vec<Vec<(usize, usize)>> = vec![Vec::new(); 2 * n];
        for (j, &(_, end, k)) in kept.iter().enumerate() {
            let (mut l, mut r) = (j + n, next[j].min(n) + n);
            while l < r {
                if l & 1 == 1 {
                    nodes[l].push((end, k));
                    l += 1;
                }
                if r & 1 == 1 {
                    r -= 1;
                    nodes[r].push((end, k));
                }
                l >>= 1;
                r >>= 1;
            }
        }
        for node in &mut nodes {
            node.sort_by_key(|&(end, _)| std::cmp::Reverse(end));
        }
        Self {
            starts: kept.iter().map(|t| t.0).collect(),
            kinds,
            nodes,
        }
    }

    /// Kinds with a tag that contains `start..end`, sorted and unique.
    fn kinds_containing(&self, start: usize, end: usize) -> Vec<&str> {
        let upto = starts_up_to(&self.starts, start);
        if upto == 0 {
            return Vec::new();
        }
        let n = self.starts.len();
        let mut hits = Vec::new();
        let mut node = upto - 1 + n;
        while node >= 1 {
            work::lookup(1);
            for &(tag_end, kind) in &self.nodes[node] {
                work::lookup(1);
                if tag_end < end {
                    break;
                }
                hits.push(kind);
            }
            node >>= 1;
        }
        hits.sort_unstable();
        hits.into_iter().map(|k| self.kinds[k].as_str()).collect()
    }
}

/// The pieces of one tag in start order, with the greatest end among the pieces up to each.
#[derive(Debug, Clone)]
struct Pieces {
    starts: Vec<usize>,
    max_end: Vec<usize>,
}

impl Pieces {
    /// The pieces at `ids` in `tags`. `ids` are in start order.
    fn build(tags: &[Tag], ids: &[usize]) -> Self {
        let mut max_end = Vec::with_capacity(ids.len());
        let mut top = 0;
        for &i in ids {
            top = top.max(tags[i].span.end);
            max_end.push(top);
        }
        Self {
            starts: ids.iter().map(|&i| tags[i].span.start).collect(),
            max_end,
        }
    }

    /// Whether one piece contains `range`.
    fn covers(&self, range: &Range<usize>) -> bool {
        let upto = starts_up_to(&self.starts, range.start);
        upto > 0 && self.max_end[upto - 1] >= range.end
    }
}

/// How many of `starts`, which are sorted, are at most `start`. Each comparison made counts
/// as one step of work.
fn starts_up_to(starts: &[usize], start: usize) -> usize {
    let mut compared = 0;
    let upto = starts.partition_point(|&s| {
        compared += 1;
        s <= start
    });
    work::lookup(compared);
    upto
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
        work::build(ids.len());
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

    /// Call `f` with the index of each tag that contains `start..end`, until `f` says false.
    fn containing(&self, start: usize, end: usize, f: &mut impl FnMut(usize) -> bool) {
        let upto = starts_up_to(&self.starts, start);
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
        f: &mut impl FnMut(usize) -> bool,
    ) -> bool {
        if lo >= upto || self.tree[node] < end {
            return true;
        }
        work::lookup(1);
        if hi - lo == 1 {
            return f(self.ids[lo]);
        }
        let mid = lo + (hi - lo) / 2;
        self.visit(2 * node, lo, mid, upto, end, f)
            && self.visit(2 * node + 1, mid, hi, upto, end, f)
    }
}

impl TagIndex {
    /// Index `tags`: sorted by span, with duplicates removed. Equal tags are not told apart.
    #[must_use]
    pub fn from_tags(mut tags: Vec<Tag>) -> Self {
        normalize_tags(&mut tags);
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
        let mut by_group: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, &g) in groups.iter().enumerate() {
            by_group.entry(g).or_default().push(i);
        }
        // A group with the same pieces, kinds included, as an earlier group answers every
        // lookup the same way, so only the first of them goes into the stabbing structures.
        // Many taggers or matches that tag one cell then cost a lookup one visit, not one each.
        let mut seen: HashSet<Vec<(&str, usize, usize)>> = HashSet::new();
        let mut repeated: HashSet<usize> = HashSet::new();
        let mut multi_groups: Vec<(usize, Vec<usize>)> =
            by_group.into_iter().filter(|(_, m)| m.len() > 1).collect();
        multi_groups.sort_unstable_by_key(|(g, _)| *g);
        for (g, m) in &multi_groups {
            // Every piece counts with its own kind, since a query can start in any piece.
            let key = m
                .iter()
                .filter_map(|&i| tags.get(i))
                .map(|t| (t.kind.as_str(), t.span.start, t.span.end))
                .collect();
            if !seen.insert(key) {
                repeated.insert(*g);
            }
        }
        let members: HashMap<usize, Pieces> = multi_groups
            .into_iter()
            .map(|(g, m)| (g, Pieces::build(&tags, &m)))
            .collect();
        let all = Spatial::build(tags.iter());
        let single = (!members.is_empty()).then(|| {
            Spatial::build(
                tags.iter()
                    .enumerate()
                    .filter(|(i, _)| !members.contains_key(&groups[*i]))
                    .map(|(_, t)| t),
            )
        });
        let multi_ids: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, g)| members.contains_key(g) && !repeated.contains(g))
            .map(|(i, _)| i)
            .collect();
        let multi = if multi_ids.is_empty() {
            Stab::default()
        } else {
            Stab::build(&tags, multi_ids)
        };
        Self {
            tags,
            groups,
            members,
            repeated,
            all,
            single,
            multi,
            by_kind: OnceLock::new(),
            budget: Budget::default(),
        }
    }

    /// The per-kind structures, built once for all lookups that name kinds.
    fn per_kind(&self) -> &HashMap<String, PerKind> {
        self.by_kind.get_or_init(|| {
            // Each tag with its index and group, kept by kind in span order.
            let mut by_kind: HashMap<&str, Vec<(usize, &Tag, usize)>> = HashMap::new();
            for (i, (t, &g)) in self.tags.iter().zip(&self.groups).enumerate() {
                by_kind.entry(t.kind.as_str()).or_default().push((i, t, g));
            }
            by_kind
                .into_iter()
                .map(|(kind, items)| {
                    let all = Spatial::build(items.iter().map(|&(_, t, _)| t));
                    let multi_ids: Vec<usize> = items
                        .iter()
                        .filter(|(_, _, g)| {
                            self.members.contains_key(g) && !self.repeated.contains(g)
                        })
                        .map(|&(i, _, _)| i)
                        .collect();
                    let (single, multi) = if multi_ids.is_empty() {
                        (None, Stab::default())
                    } else {
                        let single = Spatial::build(
                            items
                                .iter()
                                .filter(|(_, _, g)| !self.members.contains_key(g))
                                .map(|&(_, t, _)| t),
                        );
                        (Some(single), Stab::build(&self.tags, multi_ids))
                    };
                    (kind.to_string(), PerKind { all, single, multi })
                })
                .collect()
        })
    }

    /// The kinds among `wanted` that hold for `span`: [`Self::kinds_at`] restricted to
    /// `wanted`, sorted and unique. A lookup costs `O(w log n)` for `w` wanted kinds,
    /// whatever other kinds the document has.
    #[must_use]
    pub fn kinds_at_among<'k>(&self, span: &Range<usize>, wanted: &[&'k str]) -> Vec<&'k str> {
        let per = self.per_kind();
        let mut out: Vec<&'k str> = wanted
            .iter()
            .copied()
            .filter(|k| {
                per.get(*k)
                    .is_some_and(|p| !p.all.kinds_containing(span.start, span.end).is_empty())
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The kinds among `wanted` that [`Self::kinds_covering`] would answer, sorted and
    /// unique. A lookup costs `O(w log n)` for `w` wanted kinds plus the visits of the
    /// several-piece tags of those kinds, whatever other kinds the document has.
    #[must_use]
    pub fn kinds_covering_among<'k>(
        &self,
        ranges: &[Range<usize>],
        wanted: &[&'k str],
    ) -> Vec<&'k str> {
        let Some(first) = ranges.first() else {
            return Vec::new();
        };
        if self.budget.exceeded() {
            return Vec::new();
        }
        let from = ranges.iter().map(|r| r.start).min().unwrap_or(first.start);
        let to = ranges.iter().map(|r| r.end).max().unwrap_or(first.end);
        let per = self.per_kind();
        let mut out: Vec<&'k str> = Vec::new();
        for &kind in wanted {
            let Some(p) = per.get(kind) else {
                continue;
            };
            let single = p.single.as_ref().unwrap_or(&p.all);
            let mut hit = !single.kinds_containing(from, to).is_empty();
            if !hit && !p.multi.ids.is_empty() {
                p.multi.containing(first.start, first.end, &mut |i| {
                    if !self.budget.charge() {
                        return false;
                    }
                    hit = self
                        .groups
                        .get(i)
                        .and_then(|g| self.members.get(g))
                        .is_some_and(|m| ranges.iter().all(|r| m.covers(r)));
                    !hit
                });
            }
            if hit {
                out.push(kind);
            }
        }
        if self.budget.exceeded() {
            return Vec::new();
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Cap the tags that [`Self::kinds_covering`] looks at, over all its lookups on this index.
    /// Once the cap is passed, lookups answer nothing and [`Self::lookup_budget_exceeded`] is
    /// true, so a caller must discard what it built from them. A lookup that finds its answer
    /// among the tags of a single piece looks at no tag and costs nothing against the cap.
    #[must_use]
    pub fn with_lookup_budget(mut self, limit: usize) -> Self {
        self.budget = Budget {
            limit,
            spent: AtomicUsize::new(0),
        };
        self
    }

    /// Whether the cap from [`Self::with_lookup_budget`] was passed.
    #[must_use]
    pub fn lookup_budget_exceeded(&self) -> bool {
        self.budget.exceeded()
    }

    /// Kinds of the tags that cover every one of `ranges`, sorted and unique. A tag covers a
    /// range when one of its pieces contains it, so ranges on different lines of one tag
    /// are covered by it. For one range this is [`Self::kinds_at`].
    #[must_use]
    pub fn kinds_covering(&self, ranges: &[Range<usize>]) -> Vec<&str> {
        let Some(first) = ranges.first() else {
            return Vec::new();
        };
        if self.budget.exceeded() {
            return Vec::new();
        }
        // A tag of one piece covers every range when it covers the least start to the
        // greatest end.
        let from = ranges.iter().map(|r| r.start).min().unwrap_or(first.start);
        let to = ranges.iter().map(|r| r.end).max().unwrap_or(first.end);
        let single = self.single.as_ref().unwrap_or(&self.all);
        let mut set: BTreeSet<&str> = single.kinds_containing(from, to).into_iter().collect();
        if !self.multi.ids.is_empty() {
            self.multi.containing(first.start, first.end, &mut |i| {
                if !self.budget.charge() {
                    return false;
                }
                let kind = self.tags[i].kind.as_str();
                if set.contains(kind) {
                    return true;
                }
                let covered = self
                    .members
                    .get(&self.groups[i])
                    .is_some_and(|p| ranges.iter().all(|r| p.covers(r)));
                if covered {
                    set.insert(kind);
                }
                true
            });
        }
        if self.budget.exceeded() {
            return Vec::new();
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
        self.all.kinds_containing(span.start, span.end)
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
