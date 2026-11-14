//! The settings a style can set and how patches merge.
//!
//! STUB(slopvac-8l4.7.5): a local stand-in for the `Settings` patch in the
//! layered config resolution (slopvac-8l4.4.2). Merge policy follows
//! `docs/contract/config.md`: maps merge by key, lists replace.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The level a rule or category reports at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Fails the run.
    Error,
    /// Reported, does not fail on its own.
    Warning,
    /// Informational.
    Suggestion,
    /// Disabled.
    Off,
}

/// Per-category dials. A bare level stands for `{ severity = <level> }`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum CategoryPatch {
    /// A bare level.
    Level(Level),
    /// The table form.
    Dials(CategoryDials),
}

/// The table form of a category entry.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryDials {
    /// Level the category reports at.
    pub severity: Option<Level>,
    /// Density budget.
    pub max_per_100_words: Option<f64>,
    /// Score weight.
    pub weight: Option<f64>,
}

/// Terms settings a style may carry.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermsPatch {
    /// Words the style lets through the spelling check. A list replaces.
    pub allow: Option<Vec<String>>,
    /// Blocklist files, relative to the style file. A list replaces.
    pub blocklists: Option<Vec<String>>,
}

/// The settings a style sets.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SettingsPatch {
    /// Rule levels by rule key.
    #[serde(default)]
    pub rules: BTreeMap<String, Level>,
    /// Category dials by category key.
    #[serde(default)]
    pub categories: BTreeMap<String, CategoryPatch>,
    /// Terms settings.
    #[serde(default)]
    pub terms: TermsPatch,
}

impl CategoryPatch {
    pub(crate) fn dials(&self) -> CategoryDials {
        match self {
            Self::Level(l) => CategoryDials {
                severity: Some(*l),
                ..CategoryDials::default()
            },
            Self::Dials(d) => d.clone(),
        }
    }
}

/// One setting value, typed so lists keep their item boundaries.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    /// A rule or category level.
    Level(Level),
    /// A number such as a weight.
    Number(f64),
    /// A list such as `terms.allow`.
    List(Vec<String>),
}

impl std::fmt::Display for SettingValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Level(l) => write!(f, "{}", format!("{l:?}").to_lowercase()),
            Self::Number(n) => write!(f, "{n}"),
            Self::List(v) => write!(f, "{v:?}"),
        }
    }
}

impl SettingsPatch {
    /// Flatten the patch to `(key, value)` pairs, one per setting.
    ///
    /// Keys are `rules.<key>`, `categories.<key>.<field>`, `terms.allow` and
    /// `terms.blocklists`.
    #[must_use]
    pub fn entries(&self) -> Vec<(String, SettingValue)> {
        let mut out = Vec::new();
        for (k, v) in &self.rules {
            out.push((format!("rules.{k}"), SettingValue::Level(*v)));
        }
        for (k, c) in &self.categories {
            let d = c.dials();
            if let Some(v) = d.severity {
                out.push((format!("categories.{k}.severity"), SettingValue::Level(v)));
            }
            if let Some(v) = d.max_per_100_words {
                out.push((
                    format!("categories.{k}.max_per_100_words"),
                    SettingValue::Number(v),
                ));
            }
            if let Some(v) = d.weight {
                out.push((format!("categories.{k}.weight"), SettingValue::Number(v)));
            }
        }
        if let Some(v) = &self.terms.allow {
            out.push(("terms.allow".to_owned(), SettingValue::List(v.clone())));
        }
        if let Some(v) = &self.terms.blocklists {
            out.push(("terms.blocklists".to_owned(), SettingValue::List(v.clone())));
        }
        out
    }
}
