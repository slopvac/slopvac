//! STUB(slopvac-8l4.12.27): the core rules the inspection commands read.
//!
//! Main has no shipped core pack loader yet, so the binary embeds the 2.x rules
//! converted to v3 category files (the `slopvac-rules` test fixtures). The swap
//! bead replaces this module with the real pack loader.

use std::path::PathBuf;

use slopvac_engine::CORE_PACK;
use slopvac_model::PackId;
use slopvac_model::RuleId;
use slopvac_rules::{CompileOptions, LoadedCategory, LoadedRule, RuleSet};

use crate::errors::CliError;

const FILES: [(&str, &str); 26] = [
    (
        "ai-residue.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ai-residue.json"),
    ),
    (
        "ai-tells-agentic.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ai-tells-agentic.json"),
    ),
    (
        "ai-tells-content-shape.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ai-tells-content-shape.json"),
    ),
    (
        "ai-tells-figurative.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ai-tells-figurative.json"),
    ),
    (
        "ai-tells-formatting.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ai-tells-formatting.json"),
    ),
    (
        "ai-tells-register.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ai-tells-register.json"),
    ),
    (
        "ai-tells-structure.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ai-tells-structure.json"),
    ),
    (
        "docs-discipline.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/docs-discipline.json"),
    ),
    (
        "orwell.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/orwell.json"),
    ),
    (
        "prose-agency.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/prose-agency.json"),
    ),
    (
        "prose-craft.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/prose-craft.json"),
    ),
    (
        "prose-discipline.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/prose-discipline.json"),
    ),
    (
        "prose-format.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/prose-format.json"),
    ),
    (
        "prose-inclusive.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/prose-inclusive.json"),
    ),
    (
        "prose-inflation.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/prose-inflation.json"),
    ),
    (
        "prose-promotion.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/prose-promotion.json"),
    ),
    (
        "prose-scope.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/prose-scope.json"),
    ),
    (
        "ste-descriptive.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ste-descriptive.json"),
    ),
    (
        "ste-nouns.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ste-nouns.json"),
    ),
    (
        "ste-practices.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ste-practices.json"),
    ),
    (
        "ste-procedural.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ste-procedural.json"),
    ),
    (
        "ste-punctuation.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ste-punctuation.json"),
    ),
    (
        "ste-safety.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ste-safety.json"),
    ),
    (
        "ste-sentences.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ste-sentences.json"),
    ),
    (
        "ste-verbs.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ste-verbs.json"),
    ),
    (
        "ste-words.json",
        include_str!("../../slopvac-rules/tests/fixtures/core-v3/ste-words.json"),
    ),
];

/// Load the core rules.
///
/// # Errors
/// `rules-unavailable` when an embedded file fails to load.
pub fn core() -> Result<RuleSet, CliError> {
    let pack = PackId::new(CORE_PACK).expect("core is a valid plugin id");
    let sources = FILES
        .iter()
        .map(|(name, text)| (PathBuf::from(name), (*text).to_owned()));
    RuleSet::from_sources(&pack, sources, &CompileOptions::default())
        .map_err(|err| CliError::new("rules-unavailable", err.to_string()))
}

/// Every rule set a run knows: the core rules and the term-casing rules. Config validation,
/// the effective-severity commands and the engine all read the same catalogue.
#[derive(Debug)]
pub struct Catalog {
    sets: Vec<RuleSet>,
}

impl Catalog {
    /// Load the core and term-casing rules.
    ///
    /// # Errors
    /// `rules-unavailable` when an embedded file fails to load, and `engine-unavailable` when
    /// the term-casing pack does not.
    pub fn load() -> Result<Self, CliError> {
        Ok(Self {
            sets: vec![core()?, crate::lint_terms::casing_rule_set()?],
        })
    }

    /// A catalogue over `sets`, the first being the core rules.
    #[must_use]
    pub fn from_sets(sets: Vec<RuleSet>) -> Self {
        Self { sets }
    }

    /// The rule sets, core first.
    #[must_use]
    pub fn sets(&self) -> &[RuleSet] {
        &self.sets
    }

    /// The rule sets, to hand to the engine.
    #[must_use]
    pub fn into_sets(self) -> Vec<RuleSet> {
        self.sets
    }

    /// Every category, core first.
    pub fn categories(&self) -> impl Iterator<Item = &LoadedCategory> {
        self.sets.iter().flat_map(|s| s.categories().iter())
    }

    /// Every rule, core first.
    pub fn rules(&self) -> impl Iterator<Item = &LoadedRule> {
        self.sets.iter().flat_map(RuleSet::rules)
    }

    /// The rule with this id.
    #[must_use]
    pub fn get(&self, id: &RuleId) -> Option<&LoadedRule> {
        self.sets.iter().find_map(|s| s.get(id))
    }

    /// The categories a config or command-line key names, with the plugin of each. A
    /// qualified key names a category of that plugin only: `terms:orwell` is not core's
    /// `orwell`. A bare key names every category with that id, so two plugins that ship the
    /// same id make it ambiguous.
    #[must_use]
    pub fn categories_named(&self, key: &str) -> Vec<(&PackId, &LoadedCategory)> {
        let (plugin, name) = match key.split_once(':') {
            Some((plugin, name)) => (Some(plugin), name),
            None => (None, key),
        };
        self.sets
            .iter()
            .filter(|s| plugin.is_none_or(|p| s.pack().as_str() == p))
            .flat_map(|s| s.categories().iter().map(move |c| (s.pack(), c)))
            .filter(|(_, c)| c.id().as_str() == name)
            .collect()
    }

    /// The rules a config or command-line key names. A qualified key names one rule. A bare
    /// key names every rule with that local id, so two plugins that ship the same id make it
    /// ambiguous.
    #[must_use]
    pub fn rules_named(&self, key: &str) -> Vec<&LoadedRule> {
        if key.contains(':') {
            return self.rules().filter(|r| r.id().as_str() == key).collect();
        }
        self.rules().filter(|r| r.id().local() == key).collect()
    }
}

/// A catalogue with one category file for each `(plugin, category)` pair. Each category
/// has one rule, `hit`.
#[cfg(test)]
pub(crate) fn catalog_of(pairs: &[(&str, &str)]) -> Catalog {
    let triples: Vec<(&str, &str, &str)> = pairs.iter().map(|(p, c)| (*p, *c, "hit")).collect();
    catalog_with_rules(&triples)
}

/// A catalogue with one category file for each `(plugin, category, rule)` triple. The
/// category has that one rule.
#[cfg(test)]
pub(crate) fn catalog_with_rules(triples: &[(&str, &str, &str)]) -> Catalog {
    let set = |plugin: &str, category: &str, rule: &str| {
        let file = serde_json::json!({
            "schema_version": 3, "id": category, "title": "t", "description": "d",
            "rules": [{
                "id": rule, "name": "n", "message": "Found {match}.",
                "severity": "warning",
                "match": {"kind": "tokens", "tokens": ["leverage"]},
                "citation": {"source": "s"}
            }],
        });
        slopvac_rules::RuleSet::from_sources(
            &slopvac_model::PackId::new(plugin).expect("a plugin id"),
            vec![(std::path::PathBuf::from("c.json"), file.to_string())],
            &slopvac_rules::CompileOptions::default(),
        )
        .expect("the rule set loads")
    };
    Catalog::from_sets(triples.iter().map(|(p, c, r)| set(p, c, r)).collect())
}
