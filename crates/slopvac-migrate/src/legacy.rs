//! The 2.x profile data that `--preserve-behavior` expands against: the
//! `core:legacy-<profile>` styles, each rule's kind and tiers, and the per-rule
//! defaults of each profile.
//!
//! `tests/fixtures/generate.py` writes the data files from
//! `docs/contract/compat/profiles/` and the 2.x ruleset.

use std::collections::{BTreeMap, BTreeSet};

use toml_edit::{DocumentMut, Item, TableLike};

/// The 2.x profiles, in the order the reference lists them.
pub(crate) const PROFILES: [&str; 3] = ["strict", "normal", "relaxed"];

const STYLES: [&str; 3] = [
    include_str!("../data/legacy/legacy-strict.toml"),
    include_str!("../data/legacy/legacy-normal.toml"),
    include_str!("../data/legacy/legacy-relaxed.toml"),
];
const CATALOGUE: &str = include_str!("../data/legacy/catalogue.toml");

/// A scalar dial value: a level, `none`, or a number.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Scalar {
    Str(String),
    Int(i64),
    Float(f64),
}

impl Scalar {
    pub(crate) fn from_item(item: &Item) -> Option<Self> {
        let v = item.as_value()?;
        if let Some(s) = v.as_str() {
            Some(Self::Str(s.to_owned()))
        } else if let Some(i) = v.as_integer() {
            Some(Self::Int(i))
        } else {
            v.as_float().map(Self::Float)
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn to_value(&self) -> toml_edit::Value {
        match self {
            Self::Str(s) => s.as_str().into(),
            Self::Int(i) => (*i).into(),
            Self::Float(f) => (*f).into(),
        }
    }
}

/// Category dials: `severity`, `minimum_severity`, `max_per_100_words`, `weight`.
pub(crate) type Dials = BTreeMap<String, Scalar>;

/// The categories and rules of one config layer, under 2.x names.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Layer {
    pub(crate) categories: BTreeMap<String, Dials>,
    pub(crate) rules: BTreeMap<String, String>,
}

/// A 2.x rule tier for one profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tier {
    Enforced,
    Advisory,
    Excluded,
}

/// What the expansion needs to know about one 2.x rule.
#[derive(Debug, Clone)]
pub(crate) struct RuleInfo {
    pub(crate) judgement: bool,
    tiers: [Tier; 3],
}

impl RuleInfo {
    pub(crate) fn tier_for(&self, profile: &str) -> Tier {
        self.tiers[profile_index(profile)]
    }
}

/// The bundled 2.x data.
#[derive(Debug)]
pub(crate) struct Legacy {
    styles: [Layer; 3],
    defaults: [BTreeMap<String, String>; 3],
    pub(crate) rules: BTreeMap<String, RuleInfo>,
    /// Categories that some legacy style gives a budget.
    pub(crate) budgeted: BTreeSet<String>,
    /// Categories that some legacy style gives a weight.
    pub(crate) weighted: BTreeSet<String>,
    /// Each category's own weight, which 2.x scored with where no profile set one.
    pub(crate) weights: BTreeMap<String, f64>,
}

fn profile_index(profile: &str) -> usize {
    PROFILES
        .iter()
        .position(|p| *p == profile)
        .expect("profile was validated against PROFILES")
}

/// The category of a 2.x qualified rule id `<category>.<rule>`.
pub(crate) fn category_of(qid: &str) -> &str {
    qid.split_once('.').map_or(qid, |(cat, _)| cat)
}

fn parse(text: &str) -> DocumentMut {
    text.parse().expect("bundled legacy data parses")
}

fn levels(t: Option<&dyn TableLike>) -> BTreeMap<String, String> {
    t.map(|t| {
        t.iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.to_owned(), s.to_owned())))
            .collect()
    })
    .unwrap_or_default()
}

pub(crate) fn read_dials(item: &Item) -> Option<Dials> {
    if let Some(level) = item.as_str() {
        return Some(Dials::from([(
            "severity".to_owned(),
            Scalar::Str(level.to_owned()),
        )]));
    }
    let t = item.as_table_like()?;
    t.iter()
        .map(|(k, v)| Scalar::from_item(v).map(|s| (k.to_owned(), s)))
        .collect()
}

fn style(text: &str) -> Layer {
    let doc = parse(text);
    let categories = doc
        .get("categories")
        .and_then(Item::as_table_like)
        .map(|t| {
            t.iter()
                .map(|(k, v)| {
                    (
                        k.to_owned(),
                        read_dials(v).expect("legacy style dials are scalars"),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Layer {
        categories,
        rules: levels(doc.get("rules").and_then(Item::as_table_like)),
    }
}

fn tier(t: &dyn TableLike, profile: &str) -> Tier {
    match t.get(profile).and_then(Item::as_str) {
        Some("enforced") => Tier::Enforced,
        Some("advisory") => Tier::Advisory,
        _ => Tier::Excluded,
    }
}

impl Legacy {
    /// The bundled data.
    pub(crate) fn bundled() -> Self {
        let styles = STYLES.map(style);
        let cat = parse(CATALOGUE);
        let defaults = PROFILES.map(|p| {
            levels(
                cat.get("defaults")
                    .and_then(|d| d.get(p))
                    .and_then(Item::as_table_like),
            )
        });
        let rules = cat
            .get("rules")
            .and_then(Item::as_table_like)
            .expect("catalogue has rules")
            .iter()
            .map(|(qid, v)| {
                let t = v.as_table_like().expect("catalogue entry is a table");
                let info = RuleInfo {
                    judgement: t.get("kind").and_then(Item::as_str) == Some("judgement"),
                    tiers: PROFILES.map(|p| tier(t, p)),
                };
                (qid.to_owned(), info)
            })
            .collect();
        let with_dial = |dial: &str| -> BTreeSet<String> {
            styles
                .iter()
                .flat_map(|s| s.categories.iter())
                .filter(|(_, d)| d.contains_key(dial))
                .map(|(name, _)| name.clone())
                .collect()
        };
        let budgeted = with_dial("max_per_100_words");
        let weighted = with_dial("weight");
        // The generator writes every weight as a float, and the M1 vectors fail on a lost one.
        let weights = cat
            .get("weights")
            .and_then(Item::as_table_like)
            .into_iter()
            .flat_map(TableLike::iter)
            .filter_map(|(name, v)| Some((name.to_owned(), v.as_float()?)))
            .collect();
        Self {
            styles,
            defaults,
            rules,
            budgeted,
            weighted,
            weights,
        }
    }

    /// The style layer of a profile.
    pub(crate) fn style(&self, profile: &str) -> &Layer {
        &self.styles[profile_index(profile)]
    }

    /// The 2.x per-rule defaults of a profile.
    pub(crate) fn defaults(&self, profile: &str) -> &BTreeMap<String, String> {
        &self.defaults[profile_index(profile)]
    }

    /// The 2.x engine state of a rule under a profile with no authored config. It
    /// equals the level of the rule under the profile's legacy style alone (contract
    /// test M1): the style names every category with a severity.
    pub(crate) fn state(&self, profile: &str, qid: &str) -> Option<String> {
        let style = self.style(profile);
        style.rules.get(qid).cloned().or_else(|| {
            style
                .categories
                .get(category_of(qid))
                .and_then(|d| d.get("severity"))
                .and_then(Scalar::as_str)
                .map(str::to_owned)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_data_names_every_rule_category_in_each_style() {
        let legacy = Legacy::bundled();
        assert!(!legacy.rules.is_empty());
        for p in PROFILES {
            for qid in legacy.rules.keys() {
                assert!(legacy.state(p, qid).is_some(), "{p}: {qid}");
            }
        }
    }
}
