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
            let wanted = ["d", "b", "absent"];
            let among = |all: Vec<&str>| -> Vec<&str> {
                wanted.iter().copied().filter(|k| all.contains(k)).collect::<BTreeSet<_>>().into_iter().collect()
            };
            prop_assert_eq!(
                index.kinds_covering_among(ranges, &wanted),
                among(oracle.kinds_covering(ranges))
            );
            for r in ranges {
                prop_assert_eq!(index.kinds_at(r), oracle.kinds_at(r));
                prop_assert_eq!(index.kinds_at_among(r, &wanted), among(oracle.kinds_at(r)));
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
/// with a finding in each sentence makes. A lookup is one binary search over the starts, at
/// most `probes(n)` comparisons, and one walk from a leaf to the root of the tree, at most
/// `probes(n)` nodes, and it reads about one entry at each node on the way. That is under
/// `2 * probes(n)` steps for two kinds and a few more entries, so a bound of `4 * probes(n)` a
/// lookup holds with room: the counts are about half of it. An index that scanned every tag
/// would cost `n` steps a lookup and fail the bound at the larger sizes.
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
        let bound = 4 * probes(n + 1) * n;
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

/// `n` sentence tags that each have a kind of their own, and a lookup for each sentence. The
/// number of kinds has no upper limit, so a lookup may not cost a step for each kind.
#[test]
fn lookups_do_not_cost_a_step_for_each_kind() {
    for n in [1_000, 2_000, 4_000, 8_000] {
        let tags: Vec<Tag> = (0..n)
            .map(|i| tag(&format!("k{i:05}"), i * 10..i * 10 + 9))
            .collect();
        let index = TagIndex::from_tags(tags);
        let _ = take_lookup_steps();
        for i in 0..n {
            let kinds = index.kinds_at(&(i * 10 + 2..i * 10 + 5));
            assert_eq!(kinds, [format!("k{i:05}")]);
        }
        let used = take_lookup_steps();
        let bound = 6 * probes(n) * n;
        assert!(used > 0, "no work counted at size {n}");
        assert!(used <= bound, "{used} steps for {n} lookups, bound {bound}");
    }
}

/// `n` tags of one kind whose spans overlap and slide: `i..n + i`. Every tag holds the middle
/// of the document, and the answer is the one kind. A lookup may not cost a step for each tag
/// that holds the span.
#[test]
fn lookups_do_not_cost_a_step_for_each_tag_of_a_kind() {
    for n in [1_000, 2_000, 4_000, 8_000] {
        let tags: Vec<Tag> = (0..n).map(|i| tag("slide", i..n + i)).collect();
        let index = TagIndex::from_tags(tags);
        let _ = take_lookup_steps();
        for _ in 0..n {
            assert_eq!(index.kinds_at(&(n / 2..n / 2 + 1)), ["slide"]);
        }
        let used = take_lookup_steps();
        let bound = 6 * probes(n) * n;
        assert!(used > 0, "no work counted at size {n}");
        assert!(used <= bound, "{used} steps for {n} lookups, bound {bound}");
    }
}

/// `kinds` tags of distinct kinds, each in two pieces that sit on the same two cells.
fn wrapped_kinds(kinds: usize) -> Vec<(Tag, usize)> {
    (0..kinds)
        .flat_map(|i| {
            let kind = format!("k{i:05}");
            [(tag(&kind, 0..10), i), (tag(&kind, 20..30), i)]
        })
        .collect()
}

/// Ranges in the first cell and past the second: every tag holds the first range, and none
/// covers both, so a lookup finds nothing, whatever it costs.
const MISSES: [Range<usize>; 2] = [2..5, 40..45];

/// Many tags on one wrapped cell and many lookups that cross its boundary cost kinds times
/// lookups when each lookup looks at each tag. The budget caps the looks over all lookups, so
/// the work stops soon after the cap, and the index says so.
#[test]
fn the_lookup_budget_stops_the_work_when_lookups_look_at_too_many_tags() {
    let (kinds, lookups, limit) = (1_000, 1_000, 10_000);
    let index = TagIndex::from_pieces(wrapped_kinds(kinds)).with_lookup_budget(limit);
    let _ = take_lookup_steps();
    for _ in 0..lookups {
        assert_eq!(index.kinds_covering(&MISSES), Vec::<&str>::new());
    }
    let used = take_lookup_steps();
    assert!(index.lookup_budget_exceeded());
    // Each look at a tag is one step of the tree walk, and the walk to it costs a few more.
    // Without a cap the steps are over a million.
    assert!(used <= 8 * limit, "{used} steps with a cap of {limit}");
}

/// Within the cap, the budget changes nothing: the answers equal the index without one.
#[test]
fn lookups_within_the_budget_answer_as_without_one() {
    let pieces = wrapped_kinds(50);
    let free = TagIndex::from_pieces(pieces.clone());
    let capped = TagIndex::from_pieces(pieces).with_lookup_budget(10_000);
    let one = 2..5;
    for ranges in [&MISSES[..], &[2..5, 22..25], std::slice::from_ref(&one)] {
        assert_eq!(capped.kinds_covering(ranges), free.kinds_covering(ranges));
    }
    assert!(!capped.lookup_budget_exceeded());
}
