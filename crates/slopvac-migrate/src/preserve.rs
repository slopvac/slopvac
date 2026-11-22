//! `--preserve-behavior`: the expansion steps E1 to E6 of docs/contract/profiles.md,
//! section 4, over 2.x names.
//!
//! Ported from `expand` in `tests/contract/preserve_expansion.py`. The caller writes
//! the result into the migrated document.

use std::collections::{BTreeMap, BTreeSet};

use crate::legacy::{Dials, Layer, Legacy, Scalar, Tier, category_of};
use crate::overlap::{Glob, Relation, UnsupportedGlob, build_regions, parse_glob, relate};

/// One 2.x override block.
#[derive(Debug, Clone)]
pub(crate) struct Block2 {
    pub(crate) files: Vec<String>,
    pub(crate) profile: Option<String>,
    pub(crate) layer: Layer,
}

/// The parts of a 2.x config that the expansion reads.
#[derive(Debug, Clone)]
pub(crate) struct Config2 {
    pub(crate) profile: String,
    pub(crate) layer: Layer,
    pub(crate) overrides: Vec<Block2>,
}

/// A 3.0 override block that the expansion adds: an E4 guard.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Block3 {
    pub(crate) files: Vec<String>,
    pub(crate) layer: Layer,
}

/// An original 2.x block at its place in the expanded `overrides`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Patched {
    /// Index of the block in the 2.x `overrides`.
    pub(crate) index: usize,
    /// The categories and rules the block holds after the expansion.
    pub(crate) layer: Layer,
    /// E3: the 2.x profile whose switch the block now holds.
    pub(crate) switch: Option<String>,
    /// E3: earlier blocks that hold every file of this one; their entries are restated.
    pub(crate) restated: Vec<usize>,
    /// E3: the later block this one now follows, which holds its files and switches
    /// them to another profile.
    pub(crate) moved_after: Option<usize>,
    /// E3: the blocks that won over this one in 2.x and now sit below it. Their other
    /// settings are removed from this block.
    pub(crate) yields_to: Vec<usize>,
    /// E4: how many rule entries of a guard with the same files the block holds.
    pub(crate) guarded: usize,
}

impl Patched {
    fn plain(index: usize, layer: Layer) -> Self {
        Self {
            index,
            layer,
            switch: None,
            restated: Vec::new(),
            moved_after: None,
            yields_to: Vec::new(),
            guarded: 0,
        }
    }
}

/// One block of the expanded `overrides`, in order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Out {
    Original(Patched),
    /// Placed after every other block (E4).
    Guard(Block3),
}

/// Why an authored rule entry was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropReason {
    /// A judgement rule takes its level from the legacy style (E4).
    Judgement,
    /// 2.x ignored the entry in every set of blocks that can match a file (E4).
    Ignored,
}

/// An authored rule entry that the expansion drops. `block` is `None` at the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DroppedRule {
    pub(crate) block: Option<usize>,
    pub(crate) rule: String,
    pub(crate) reason: DropReason,
}

/// The expansion result.
#[derive(Debug, Clone)]
pub(crate) struct Expanded {
    /// The top-level layer after E2, E4 and E5.
    pub(crate) top: Layer,
    /// The override blocks in their new order (E3), then the guards (E4).
    pub(crate) overrides: Vec<Out>,
    pub(crate) dropped: Vec<DroppedRule>,
}

fn severity(d: &Dials) -> Option<&str> {
    d.get("severity").and_then(Scalar::as_str)
}

/// E5: an authored category without `severity` gets the severity of the profile in
/// force.
fn authored(
    legacy: &Legacy,
    cats: &BTreeMap<String, Dials>,
    profile: &str,
) -> BTreeMap<String, Dials> {
    cats.iter()
        .map(|(name, d)| {
            let mut d = d.clone();
            if !d.contains_key("severity")
                && let Some(level) = legacy
                    .style(profile)
                    .categories
                    .get(name)
                    .and_then(|s| s.get("severity"))
            {
                d.insert("severity".to_owned(), level.clone());
            }
            (name.clone(), d)
        })
        .collect()
}

/// E2: the profile defaults of each authored category that the layer does not set.
fn sticky_for(
    legacy: &Legacy,
    categories: &BTreeMap<String, Dials>,
    profile: &str,
    own: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    legacy
        .defaults(profile)
        .iter()
        .filter(|(qid, _)| categories.contains_key(category_of(qid)) && !own.contains_key(*qid))
        .map(|(qid, level)| (qid.clone(), level.clone()))
        .collect()
}

struct Cx<'a> {
    legacy: &'a Legacy,
    config: &'a Config2,
    guards: BTreeMap<Vec<usize>, BTreeMap<String, String>>,
}

impl Cx<'_> {
    /// The final profile and the category layers that a set of blocks applies.
    fn effective(&self, scope: &BTreeSet<usize>) -> (&str, Vec<&BTreeMap<String, Dials>>) {
        let mut profile = self.config.profile.as_str();
        let mut cats = vec![&self.config.layer.categories];
        for &i in scope {
            let block = &self.config.overrides[i];
            if let Some(p) = &block.profile {
                profile = p;
            }
            cats.push(&block.layer.categories);
        }
        (profile, cats)
    }

    /// Whether 2.x ignored an entry for `qid` in the files of `scope`.
    fn ignored(&self, qid: &str, scope: &BTreeSet<usize>) -> bool {
        let (profile, cats) = self.effective(scope);
        if self.legacy.rules[qid].tier_for(profile) == Tier::Excluded {
            return true;
        }
        let category = category_of(qid);
        let mut level = self
            .legacy
            .style(profile)
            .categories
            .get(category)
            .and_then(severity);
        for layer in cats {
            if let Some(l) = layer.get(category).and_then(severity) {
                level = Some(l);
            }
        }
        level == Some("off")
    }

    /// E4: keep an entry unless 2.x ignored it in every visible set; guard the sets
    /// where it did.
    fn keep(&mut self, qid: &str, visible: &[&BTreeSet<usize>]) -> bool {
        let flags: Vec<bool> = visible.iter().map(|s| self.ignored(qid, s)).collect();
        if flags.iter().all(|f| *f) {
            return false;
        }
        for (scope, flag) in visible.iter().zip(flags) {
            if flag {
                self.guards
                    .entry(scope.iter().copied().collect())
                    .or_default()
                    .insert(qid.to_owned(), "off".to_owned());
            }
        }
        true
    }

    /// The checked rule entries of a layer that survive E4.
    fn kept_rules(
        &mut self,
        block: Option<usize>,
        rules: &BTreeMap<String, String>,
        visible: &[&BTreeSet<usize>],
        dropped: &mut Vec<DroppedRule>,
    ) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (qid, level) in rules {
            let reason = if self.legacy.rules[qid].judgement {
                Some(DropReason::Judgement)
            } else if self.keep(qid, visible) {
                None
            } else {
                Some(DropReason::Ignored)
            };
            match reason {
                None => {
                    out.insert(qid.clone(), level.clone());
                }
                Some(reason) => dropped.push(DroppedRule {
                    block,
                    rule: qid.clone(),
                    reason,
                }),
            }
        }
        out
    }
}

/// Expand a 2.x config. Every rule and category name must be in `legacy`, and every
/// profile one of [`crate::legacy::PROFILES`]; the caller checks that first.
pub(crate) fn expand(config: &Config2, legacy: &Legacy) -> Result<Expanded, UnsupportedGlob> {
    let top = config.profile.as_str();
    let files: Vec<Vec<String>> = config.overrides.iter().map(|b| b.files.clone()).collect();
    let regions = build_regions(&files)?;
    let globs: Vec<Glob> = files
        .iter()
        .enumerate()
        .map(|(block, f)| parse_glob(f).map_err(|reason| UnsupportedGlob { block, reason }))
        .collect::<Result<_, _>>()?;
    let scopes: Vec<&BTreeSet<usize>> = regions.iter().map(|r| &r.scope).collect();
    let mut cx = Cx {
        legacy,
        config,
        guards: BTreeMap::new(),
    };
    let mut dropped = Vec::new();
    let top_real = &config.layer.categories;

    // E1, E2, E4, E5 at the top level.
    let top_cat = authored(legacy, top_real, top);
    let top_rules = cx.kept_rules(None, &config.layer.rules, &scopes, &mut dropped);
    let mut rules = sticky_for(legacy, &top_cat, top, &top_rules);
    rules.extend(top_rules.clone());
    let top_layer = Layer {
        categories: top_cat,
        rules,
    };

    // E2, E4, E5 in each block, under the profile the block selects or the top one.
    let mut kept = Vec::new();
    let mut blocks = Vec::new();
    for (index, block) in config.overrides.iter().enumerate() {
        let in_force = block.profile.as_deref().unwrap_or(top);
        let cats = authored(legacy, &block.layer.categories, in_force);
        let visible: Vec<&BTreeSet<usize>> = scopes
            .iter()
            .copied()
            .filter(|s| s.contains(&index))
            .collect();
        let own = cx.kept_rules(Some(index), &block.layer.rules, &visible, &mut dropped);
        let mut rules = sticky_for(legacy, &cats, in_force, &own);
        rules.extend(own.clone());
        kept.push(own);
        blocks.push(Layer {
            categories: cats,
            rules,
        });
    }

    let e3 = E3 {
        legacy,
        config,
        top_rules: &top_rules,
        globs: &globs,
        kept: &kept,
        blocks: &blocks,
    };
    let mut placed = Vec::new();
    for index in 0..config.overrides.len() {
        e3.place(index, &mut placed);
    }
    // `keep` records a guard only under a scope from `scopes`, so each one is a region's.
    let mut pending = cx.guards;
    let guards: BTreeMap<Vec<usize>, Block3> = regions
        .iter()
        .filter_map(|region| {
            let scope: Vec<usize> = region.scope.iter().copied().collect();
            let rules = pending.remove(&scope)?;
            let guard = Block3 {
                files: region.files.clone(),
                layer: Layer {
                    categories: BTreeMap::new(),
                    rules,
                },
            };
            Some((scope, guard))
        })
        .collect();
    let mut unfolded = Vec::new();
    for guard in guards.into_values() {
        if !fold_guard(&guard, &globs, &mut placed) {
            unfolded.push(Out::Guard(guard));
        }
    }
    let mut overrides: Vec<Out> = placed.into_iter().map(Out::Original).collect();
    overrides.extend(unfolded);

    Ok(Expanded {
        top: top_layer,
        overrides,
        dropped,
    })
}

/// E4: put a guard into the last block with the same files, where that is exact: no
/// block after it that contains or equals those files states a rule the guard turns
/// off, or the `severity` of that rule's category, because either would outrank the
/// merged `off`. The loader refuses two blocks with the same files. Returns whether the
/// guard was folded.
fn fold_guard(guard: &Block3, globs: &[Glob], placed: &mut [Patched]) -> bool {
    let Ok(glob) = parse_glob(&guard.files) else {
        return false;
    };
    let Some(at) = placed.iter().rposition(|b| globs[b.index] == glob) else {
        return false;
    };
    let revived = placed[at + 1..].iter().any(|b| {
        matches!(
            relate(&globs[b.index], &glob),
            Relation::Contains | Relation::Equal
        ) && guard.layer.rules.keys().any(|qid| {
            b.layer.rules.contains_key(qid)
                || b.layer
                    .categories
                    .get(category_of(qid))
                    .is_some_and(|d| d.contains_key("severity"))
        })
    });
    if revived {
        return false;
    }
    let block = &mut placed[at];
    block.guarded += guard.layer.rules.len();
    block.layer.rules.extend(
        guard
            .layer
            .rules
            .iter()
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    true
}

/// Lay `top` over `base`: per category dial and per rule, `top` wins.
fn merge_into(base: &mut Layer, top: &Layer) {
    for (name, d) in &top.categories {
        base.categories
            .entry(name.clone())
            .or_default()
            .extend(d.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    base.rules
        .extend(top.rules.iter().map(|(k, v)| (k.clone(), v.clone())));
}

/// Make `layer`, which now sits above `over`, give way to it as it did in 2.x: drop
/// every dial and rule that `over` sets, then restate the rules of `over` in each
/// category that `layer` still levels, because a category entry in a higher block beats
/// a rule entry in a lower one.
fn yield_to(layer: &mut Layer, over: &Layer) {
    for (name, d) in &over.categories {
        if let Some(mine) = layer.categories.get_mut(name) {
            mine.retain(|k, _| !d.contains_key(k));
        }
    }
    layer.categories.retain(|_, d| !d.is_empty());
    layer.rules.retain(|qid, _| !over.rules.contains_key(qid));
    for (qid, level) in &over.rules {
        if layer
            .categories
            .get(category_of(qid))
            .is_some_and(|d| d.contains_key("severity"))
        {
            layer.rules.insert(qid.clone(), level.clone());
        }
    }
}

/// E3: put each profile switch into the block that selects the profile.
struct E3<'a> {
    legacy: &'a Legacy,
    config: &'a Config2,
    top_rules: &'a BTreeMap<String, String>,
    globs: &'a [Glob],
    kept: &'a [BTreeMap<String, String>],
    blocks: &'a [Layer],
}

impl E3<'_> {
    /// The authored entries of block `j` for files where 2.x applied `profile`: E5 and
    /// E2 under that profile, and the rule entries that survive E4.
    fn under(&self, j: usize, profile: &str) -> Layer {
        let cats = authored(
            self.legacy,
            &self.config.overrides[j].layer.categories,
            profile,
        );
        let mut rules = sticky_for(self.legacy, &cats, profile, &self.kept[j]);
        rules.extend(self.kept[j].clone());
        Layer {
            categories: cats,
            rules,
        }
    }

    fn category_names(&self, blocks: impl IntoIterator<Item = usize>) -> BTreeSet<&str> {
        blocks
            .into_iter()
            .flat_map(|j| self.config.overrides[j].layer.categories.keys())
            .map(String::as_str)
            .collect()
    }

    /// Add block `index` to `out`, which holds the blocks before it in their new order.
    ///
    /// A block without a profile goes last. A block with a profile holds its switch,
    /// with its own entries over it. An earlier block that holds every file of this
    /// one applied to these files under the new profile, so its entries are restated
    /// here, below this block's own. An earlier block that lies inside this one moves
    /// to just after it, recomputed under the new profile, and gives way to the
    /// entries that beat it in 2.x.
    fn place(&self, index: usize, out: &mut Vec<Patched>) {
        let Some(profile) = self.config.overrides[index].profile.as_deref() else {
            out.push(Patched::plain(index, self.blocks[index].clone()));
            return;
        };
        let glob = &self.globs[index];
        let containers: Vec<usize> = (0..index)
            .filter(|&j| {
                matches!(
                    relate(&self.globs[j], glob),
                    Relation::Contains | Relation::Equal
                )
            })
            .collect();
        let nested: BTreeSet<usize> = (0..index)
            .filter(|&j| relate(&self.globs[j], glob) == Relation::Contained)
            .collect();
        let restated: BTreeMap<usize, Layer> = containers
            .iter()
            .map(|&j| (j, self.under(j, profile)))
            .collect();

        let own = self.category_names(containers.iter().copied().chain([index]));
        let inner = self.category_names(nested.iter().copied());
        let mut layer = switch_layer(
            self.legacy,
            self.config,
            self.top_rules,
            profile,
            &own,
            &inner,
        );
        for r in restated.values() {
            merge_into(&mut layer, r);
        }
        merge_into(&mut layer, &self.blocks[index]);

        let (moved, stay): (Vec<Patched>, Vec<Patched>) = std::mem::take(out)
            .into_iter()
            .partition(|b| nested.contains(&b.index));
        *out = stay;
        out.push(Patched {
            index,
            layer,
            switch: Some(profile.to_owned()),
            restated: containers.clone(),
            moved_after: None,
            yields_to: Vec::new(),
            guarded: 0,
        });
        for b in moved {
            let j = b.index;
            // The blocks that beat j in 2.x: those an earlier move put above it, the
            // containers of this block after j, and this block. In 2.x order, so the
            // last one wins where two set the same key.
            let winners: BTreeSet<usize> = b
                .yields_to
                .iter()
                .copied()
                .chain(containers.iter().copied().filter(|&k| k > j))
                .chain([index])
                .collect();
            let mut over = Layer::default();
            for &k in &winners {
                let layer = if k == index {
                    self.blocks[index].clone()
                } else {
                    restated
                        .get(&k)
                        .cloned()
                        .unwrap_or_else(|| self.under(k, profile))
                };
                merge_into(&mut over, &layer);
            }
            let mut layer = self.under(j, profile);
            yield_to(&mut layer, &over);
            out.push(Patched {
                index: j,
                layer,
                switch: None,
                restated: Vec::new(),
                moved_after: Some(index),
                yields_to: winners.into_iter().collect(),
                guarded: 0,
            });
        }
    }
}

/// E3 and E6: the entries that switch a set of files to `profile`.
///
/// `own` holds the categories that the block holding the switch authors or restates.
/// 2.x applied an authored category level over the advisory demotion and over the
/// profile's category level, so the switch states no rule entry for such a rule unless
/// the profile excludes the rule or gives it a default. `inner` holds the categories
/// that a block placed after this one authors. A rule entry that only repeats the
/// profile's category level would hold against such a category where it is `off`, so
/// the switch states only the rule entries of the style there.
fn switch_layer(
    legacy: &Legacy,
    config: &Config2,
    top_rules: &BTreeMap<String, String>,
    profile: &str,
    own: &BTreeSet<&str>,
    inner: &BTreeSet<&str>,
) -> Layer {
    let top_real = &config.layer.categories;
    let empty = Dials::new();
    let mut categories = BTreeMap::new();
    for (name, d) in &legacy.style(profile).categories {
        let authored = top_real.get(name).unwrap_or(&empty);
        let mut kept: Dials = d
            .iter()
            .filter(|(k, _)| !authored.contains_key(*k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if !d.contains_key("max_per_100_words")
            && legacy.budgeted.contains(name)
            && !authored.contains_key("max_per_100_words")
        {
            kept.insert(
                "max_per_100_words".to_owned(),
                Scalar::Str("none".to_owned()),
            );
        }
        // 2.x scored a category with no profile weight at the category's own weight.
        if !d.contains_key("weight")
            && legacy.weighted.contains(name)
            && !authored.contains_key("weight")
            && let Some(w) = legacy.weights.get(name)
        {
            kept.insert("weight".to_owned(), Scalar::Float(*w));
        }
        if !kept.is_empty() {
            categories.insert(name.clone(), kept);
        }
    }
    let mut keys: BTreeSet<&String> = BTreeSet::new();
    for p in crate::legacy::PROFILES {
        keys.extend(legacy.style(p).rules.keys());
    }
    keys.extend(config.layer.rules.keys());
    let mut rules = BTreeMap::new();
    for qid in keys {
        if let Some(level) = top_rules.get(qid) {
            rules.insert(qid.clone(), level.clone());
            continue;
        }
        let info = &legacy.rules[qid];
        let category = category_of(qid);
        if !info.judgement {
            let excluded = info.tier_for(profile) == Tier::Excluded;
            let default = legacy.defaults(profile).contains_key(qid);
            if own.contains(category) && !excluded && !default {
                continue;
            }
            if inner.contains(category) && !legacy.style(profile).rules.contains_key(qid) {
                continue;
            }
        }
        if top_real.contains_key(category)
            && !info.judgement
            && info.tier_for(profile) == Tier::Advisory
            && !legacy.defaults(profile).contains_key(qid)
        {
            continue;
        }
        if let Some(state) = legacy.state(profile, qid) {
            rules.insert(qid.clone(), state);
        }
    }
    Layer { categories, rules }
}
