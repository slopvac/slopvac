//! Override glob grammar for `--preserve-behavior` (docs/contract/profiles.md, section 4).
//!
//! The expansion must know which override blocks a file can match together. It
//! accepts one pattern per block, an anchored literal path such as `docs/api` or a
//! tree such as `docs/**`, so the blocks form a containment forest. Any other
//! pattern stops the migration, because the overlaps cannot be decided.
//!
//! Ported from `parse_glob`, `relate` and `build_regions` in
//! `tests/contract/preserve_expansion.py`.

use std::collections::BTreeSet;

use crate::messages::MigrateText;

/// One accepted override pattern: its path segments and whether it is a tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Glob {
    /// The path segments, without the trailing `**` of a tree.
    pub segments: Vec<String>,
    /// `docs/**` is a tree: the proper descendants of `docs`. `docs` is a literal
    /// path, which gitignore matches together with everything below it.
    pub tree: bool,
}

/// How the files of two globs relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// The globs match the same files.
    Equal,
    /// The first glob matches every file of the second, and more.
    Contains,
    /// The first glob matches a subset of the files of the second.
    Contained,
    /// Neither glob holds the other.
    Disjoint,
}

/// An override block whose pattern is outside the grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedGlob {
    /// Index of the block in `overrides`.
    pub block: usize,
    /// What is wrong with the pattern.
    pub reason: String,
}

/// A set of files that one set of override blocks matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// The indices of the blocks that match every file of the region.
    pub scope: BTreeSet<usize>,
    /// A gitignore `files` list for the region: the node, minus its child nodes.
    pub files: Vec<String>,
}

/// Parse the `files` list of one block. It must hold exactly one pattern that
/// contains a slash. No segment may be empty, hold a wildcard, `!`, `#` or a
/// backslash, or be `.` or `..`.
pub fn parse_glob(files: &[String]) -> Result<Glob, String> {
    let [glob] = files else {
        return Err(MigrateText::OverlapManyPatterns {
            count: files.len(),
            patterns: format!("{files:?}"),
        }
        .to_string());
    };
    let unsupported = || {
        MigrateText::OverlapUnsupported {
            pattern: format!("{glob:?}"),
        }
        .to_string()
    };
    let (body, tree) = match glob.strip_suffix("/**") {
        Some(body) => (body, true),
        None => (glob.as_str(), false),
    };
    if !glob.contains('/') {
        return Err(unsupported());
    }
    let segments: Vec<String> = body.split('/').map(str::to_owned).collect();
    let bad = |s: &str| {
        s.is_empty()
            || s == "."
            || s == ".."
            || s.contains(['*', '?', '[', ']', '\\', '!', '#', '/'])
    };
    if segments.iter().any(|s| bad(s)) {
        return Err(unsupported());
    }
    Ok(Glob { segments, tree })
}

/// Whether every path that `b` matches is also matched by `a`, for `a != b`.
fn holds(a: &Glob, b: &Glob) -> bool {
    if a == b || !b.segments.starts_with(&a.segments) {
        return false;
    }
    if a.tree {
        return b.segments.len() > a.segments.len();
    }
    true
}

/// How the files of `a` relate to those of `b`.
#[must_use]
pub fn relate(a: &Glob, b: &Glob) -> Relation {
    if a == b {
        Relation::Equal
    } else if holds(a, b) {
        Relation::Contains
    } else if holds(b, a) {
        Relation::Contained
    } else {
        Relation::Disjoint
    }
}

/// The regions that the blocks cut out, given each block's `files` list.
///
/// Equal globs form one node. A node's parent is the smallest node that holds it,
/// so the nodes form a forest. A region is a node minus its children, plus the root
/// region of the files that no block matches: nodes + 1 regions. The root region
/// comes first, then one per node in the order of the first block that names it.
pub fn build_regions(blocks: &[Vec<String>]) -> Result<Vec<Region>, UnsupportedGlob> {
    let mut nodes: Vec<(Glob, Vec<usize>)> = Vec::new();
    for (index, files) in blocks.iter().enumerate() {
        let key = parse_glob(files).map_err(|reason| UnsupportedGlob {
            block: index,
            reason,
        })?;
        match nodes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(index),
            None => nodes.push((key, vec![index])),
        }
    }
    let parent: Vec<Option<usize>> = (0..nodes.len())
        .map(|n| {
            let holders: Vec<usize> = (0..nodes.len())
                .filter(|&o| o != n && relate(&nodes[o].0, &nodes[n].0) == Relation::Contains)
                .collect();
            holders.iter().copied().find(|&h| {
                holders
                    .iter()
                    .all(|&o| o == h || holds(&nodes[o].0, &nodes[h].0))
            })
        })
        .collect();
    let glob_of = |n: usize| blocks[nodes[n].1[0]][0].clone();
    let children = |up: Option<usize>| -> Vec<String> {
        (0..nodes.len())
            .filter(|&n| parent[n] == up)
            .map(|n| format!("!{}", glob_of(n)))
            .collect()
    };
    let chain = |mut n: usize| -> BTreeSet<usize> {
        let mut members = BTreeSet::new();
        loop {
            members.extend(nodes[n].1.iter().copied());
            match parent[n] {
                Some(up) => n = up,
                None => return members,
            }
        }
    };
    let mut regions = vec![Region {
        scope: BTreeSet::new(),
        files: std::iter::once("**".to_owned())
            .chain(children(None))
            .collect(),
    }];
    for n in 0..nodes.len() {
        regions.push(Region {
            scope: chain(n),
            files: std::iter::once(glob_of(n))
                .chain(children(Some(n)))
                .collect(),
        });
    }
    Ok(regions)
}
