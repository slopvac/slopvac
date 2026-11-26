//! The term-casing rule: `terms.casing`, a vocabulary rule whose check is `term-casing`.
//!
//! The rule data ships here ([`terms_rule_set`]) and the check is
//! [`slopvac_terms::TermSet::check_casing`]. [`crate::Engine::with_terms`] sets the terms it
//! enforces; without a call the engine enforces the built-in list.

use std::path::PathBuf;

use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, LoadErrors, RuleSet, VocabularyMatch};

/// The `check` name of the vocabulary rule that runs the term casing check.
pub(crate) const CHECK: &str = "term-casing";

const PACK: &str = include_str!("../data/terms-pack.json");

/// The term rule pack as a rule set under `pack`: `terms.casing`, which is `core:terms.casing` under the core pack id [`crate::CORE_PACK`].
///
/// Pass this set to [`crate::Engine::new`] with the other sets. The engine runs the rule on
/// the prose of each document, never inside code, and applies its level, category and
/// suppression settings like any other rule. A finding carries a fix that spells the term
/// the canonical way.
///
/// # Errors
/// The shipped pack fails to load, which the tests rule out.
pub fn terms_rule_set(pack: &PackId) -> Result<RuleSet, LoadErrors> {
    RuleSet::from_sources(
        pack,
        [(PathBuf::from("terms.json"), PACK.to_owned())],
        &CompileOptions::default(),
    )
}

/// Whether a vocabulary rule runs the term casing check.
pub(crate) fn is_casing(m: &VocabularyMatch) -> bool {
    m.check.as_str() == CHECK
}
