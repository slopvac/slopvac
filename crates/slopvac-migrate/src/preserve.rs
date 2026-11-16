//! `--preserve-behavior`: the expansion steps E1 to E6 of docs/contract/profiles.md,
//! section 4, over 2.x names.
//!
//! Ported from `expand` in `tests/contract/preserve_expansion.py`. The caller writes
//! the result into the migrated document.

use std::collections::{BTreeMap, BTreeSet};

use crate::legacy::{Dials, Layer, Legacy, Scalar, Tier, category_of};
use crate::overlap::{UnsupportedGlob, build_regions};

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

/// A 3.0 override block that the expansion writes or patches.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Block3 {
    pub(crate) files: Vec<String>,
    pub(crate) layer: Layer,
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
    /// One block per profile override, placed before every other block (E3).
    pub(crate) switches: Vec<Block3>,
    /// The original blocks after E2, E4 and E5, in order.
    pub(crate) blocks: Vec<Layer>,
    /// Guard blocks placed after every other block (E4).
    pub(crate) guards: Vec<Block3>,
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
#[expect(
    clippy::expect_used,
    reason = "every guard scope was built from a region scope in this function"
)]
pub(crate) fn expand(config: &Config2, legacy: &Legacy) -> Result<Expanded, UnsupportedGlob> {
    let top = config.profile.as_str();
    let files: Vec<Vec<String>> = config.overrides.iter().map(|b| b.files.clone()).collect();
    let regions = build_regions(&files)?;
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

    let mut blocks = Vec::new();
    let mut switches = Vec::new();
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
        rules.extend(own);
        blocks.push(Layer {
            categories: cats,
            rules,
        });
        if let Some(profile) = &block.profile {
            switches.push(Block3 {
                files: block.files.clone(),
                layer: switch_layer(legacy, config, &top_rules, profile),
            });
        }
    }

    let guards = cx
        .guards
        .into_iter()
        .map(|(scope, rules)| {
            let set: BTreeSet<usize> = scope.into_iter().collect();
            let region = regions
                .iter()
                .find(|r| r.scope == set)
                .expect("a guard scope is a region scope");
            Block3 {
                files: region.files.clone(),
                layer: Layer {
                    categories: BTreeMap::new(),
                    rules,
                },
            }
        })
        .collect();

    Ok(Expanded {
        top: top_layer,
        switches,
        blocks,
        guards,
        dropped,
    })
}

/// E3 and E6: the block that switches a set of files to `profile`.
fn switch_layer(
    legacy: &Legacy,
    config: &Config2,
    top_rules: &BTreeMap<String, String>,
    profile: &str,
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
        if top_real.contains_key(category_of(qid))
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
