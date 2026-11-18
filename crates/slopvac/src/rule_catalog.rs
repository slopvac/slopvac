//! STUB(slopvac-8l4.12.27): the core rules the inspection commands read.
//!
//! Main has no shipped core pack loader yet, so the binary embeds the 2.x rules
//! converted to v3 category files (the `slopvac-rules` test fixtures). The swap
//! bead replaces this module with the real pack loader.

use std::path::PathBuf;

use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, RuleSet};

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
    let pack = PackId::new("core").expect("core is a valid pack id");
    let sources = FILES
        .iter()
        .map(|(name, text)| (PathBuf::from(name), (*text).to_owned()));
    RuleSet::from_sources(&pack, sources, &CompileOptions::default())
        .map_err(|err| CliError::new("rules-unavailable", err.to_string()))
}
