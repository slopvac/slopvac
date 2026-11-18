//! The tag index answers context lookups as the plain scan over every tag did, and does work
//! that grows with the number of lookups, not with lookups times tags.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use proptest::prelude::*;

use slopvac_context::{Tag, TagIndex, take_lookup_steps};

fn tag(kind: &str, span: Range<usize>) -> Tag {
    Tag {
        kind: kind.to_string(),
        span,
        confidence: 1.0,
        evidence: String::new(),
    }
}

/// The index as it was before lookups were indexed: every lookup scans every tag. The bodies
/// are the old ones, kept as the oracle.
struct Scan {
    tags: Vec<Tag>,
    groups: Vec<usize>,
    members: HashMap<usize, Vec<usize>>,
}

impl Scan {
    fn from_pieces(mut pieces: Vec<(Tag, usize)>) -> Self {
        pieces.sort_by(|(a, ga), (b, gb)| {
            (a.span.start, a.span.end, &a.kind, ga).cmp(&(b.span.start, b.span.end, &b.kind, gb))
        });
        pieces.dedup_by(|(a, ga), (b, gb)| a.kind == b.kind && a.span == b.span && ga == gb);
        let (tags, groups): (Vec<Tag>, Vec<usize>) = pieces.into_iter().unzip();
        let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
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

    fn kinds_covering(&self, ranges: &[Range<usize>]) -> Vec<&str> {
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

    fn kinds_at(&self, span: &Range<usize>) -> Vec<&str> {
        let set: BTreeSet<&str> = self
            .tags
            .iter()
            .filter(|t| t.span.start <= span.start && span.end <= t.span.end)
            .map(|t| t.kind.as_str())
            .collect();
        set.into_iter().collect()
    }

    fn kinds_seen(&self) -> Vec<&str> {
        let set: BTreeSet<&str> = self.tags.iter().map(|t| t.kind.as_str()).collect();
        set.into_iter().collect()
    }
}

fn span() -> impl Strategy<Value = Range<usize>> {
    // Includes empty and reversed ranges: lookups take whatever a finding carries.
    (0usize..30, 0usize..30).prop_map(|(a, b)| a..b)
}

fn piece() -> impl Strategy<Value = (Tag, usize)> {
    (
        prop::sample::select(vec!["a", "b", "c", "d"]),
        span(),
        0usize..6,
    )
        .prop_map(|(kind, s, group)| (tag(kind, s), group))
}

proptest! {
    #[test]
    fn lookups_equal_the_scan_for_pieces(
        pieces in prop::collection::vec(piece(), 0..24),
        queries in prop::collection::vec(prop::collection::vec(span(), 0..4), 0..12),
    ) {
        let oracle = Scan::from_pieces(pieces.clone());
        let index = TagIndex::from_pieces(pieces);
        prop_assert_eq!(index.tags(), oracle.tags.as_slice());
        prop_assert_eq!(index.kinds_seen(), oracle.kinds_seen());
        for ranges in &queries {
            prop_assert_eq!(index.kinds_covering(ranges), oracle.kinds_covering(ranges));
            for r in ranges {
                prop_assert_eq!(index.kinds_at(r), oracle.kinds_at(r));
            }
        }
    }

    #[test]
    fn lookups_equal_the_scan_for_whole_tags(
        tags in prop::collection::vec(piece(), 0..24),
        queries in prop::collection::vec(prop::collection::vec(span(), 1..4), 0..12),
    ) {
        let tags: Vec<Tag> = tags.into_iter().map(|(t, _)| t).collect();
        let mut sorted = tags.clone();
        sorted.sort_by(|a, b| {
            (a.span.start, a.span.end, &a.kind).cmp(&(b.span.start, b.span.end, &b.kind))
        });
        sorted.dedup_by(|a, b| a.kind == b.kind && a.span == b.span);
        let oracle = Scan::from_pieces(
            sorted.into_iter().enumerate().map(|(i, t)| (t, i)).collect(),
        );
        let index = TagIndex::from_tags(tags);
        for ranges in &queries {
            prop_assert_eq!(index.kinds_covering(ranges), oracle.kinds_covering(ranges));
            prop_assert_eq!(index.kinds_at(&ranges[0]), oracle.kinds_at(&ranges[0]));
        }
    }
}

/// `n.ilog2() + 1`: the probes of one binary search over `n` entries.
fn probes(n: usize) -> usize {
    n.ilog2() as usize + 1
}

/// `n` sentence tags in a document-wide section: one lookup for each sentence, as a document
/// with a finding in each sentence makes. Two kinds are present, and a lookup probes each
/// kind once, so a lookup costs at most `2 * probes(n)` steps. An index that scanned every
/// tag would cost `n` steps a lookup and fail the bound at the larger sizes.
#[test]
fn one_lookup_for_each_sentence_takes_n_log_n_steps() {
    for n in [1_000, 2_000, 4_000, 8_000] {
        let mut tags = vec![tag("section", 0..n * 10)];
        tags.extend((0..n).map(|i| tag("sentence", i * 10..i * 10 + 9)));
        let index = TagIndex::from_tags(tags);
        let _ = take_lookup_steps();
        for i in 0..n {
            assert_eq!(
                index.kinds_at(&(i * 10 + 2..i * 10 + 5)),
                ["section", "sentence"]
            );
        }
        let used = take_lookup_steps();
        let bound = 2 * probes(n + 1) * n;
        assert!(used > 0, "no work counted at size {n}");
        assert!(used <= bound, "{used} steps for {n} lookups, bound {bound}");
    }
}

/// `n` tags split over two pieces each, with a lookup for each tag's two ranges. Only the
/// tag the ranges lie in covers them, so a lookup visits the path to one tag.
#[test]
fn one_covering_lookup_for_each_split_tag_takes_n_log_n_steps() {
    for n in [1_000, 2_000, 4_000, 8_000] {
        let pieces: Vec<(Tag, usize)> = (0..n)
            .flat_map(|i| {
                let at = i * 20;
                [
                    (tag("cell", at..at + 5), i),
                    (tag("cell", at + 10..at + 15), i),
                ]
            })
            .collect();
        let index = TagIndex::from_pieces(pieces);
        let _ = take_lookup_steps();
        for i in 0..n {
            let at = i * 20;
            assert_eq!(
                index.kinds_covering(&[at + 1..at + 2, at + 11..at + 12]),
                ["cell"]
            );
        }
        let used = take_lookup_steps();
        // Per lookup: the single-piece probe, the multi-piece search, and one root-to-leaf
        // path of at most `probes(2n) + 1` nodes.
        let bound = 4 * probes(2 * n) * n;
        assert!(used > 0, "no work counted at size {n}");
        assert!(used <= bound, "{used} steps for {n} lookups, bound {bound}");
    }
}

/// One tag of `n` pieces, as a long wrapped sentence makes, and a lookup for each piece whose
/// ranges lie in that piece and in the last one. Deciding whether the tag covers the ranges
/// must not read all the pieces for each range.
#[test]
fn lookups_in_one_tag_of_many_pieces_take_n_log_n_steps() {
    for n in [1_000, 2_000, 4_000, 8_000] {
        let pieces: Vec<(Tag, usize)> = (0..n)
            .map(|i| (tag("cell", i * 10..i * 10 + 5), 0))
            .collect();
        let index = TagIndex::from_pieces(pieces);
        let last = (n - 1) * 10 + 1;
        let _ = take_lookup_steps();
        for i in 0..n {
            let at = i * 10 + 1;
            assert_eq!(
                index.kinds_covering(&[at..at + 1, last..last + 1]),
                ["cell"]
            );
        }
        let used = take_lookup_steps();
        // Per lookup: the multi-piece search, the path to one piece, and a search of the
        // pieces for each of two ranges.
        let bound = 6 * probes(n) * n;
        assert!(used > 0, "no work counted at size {n}");
        assert!(used <= bound, "{used} steps for {n} lookups, bound {bound}");
    }
}
