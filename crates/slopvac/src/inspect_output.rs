//! The documents of `slopvac rules --format json` and `slopvac explain --format json`.
//!
//! The types are the producer contract of `schemas/v3/rules-output.schema.json`, which
//! `slopvac-schemas` generates from them. Field order is the JSON key order: the members of
//! the rule definition, then the ones the binary adds. Semantics are in
//! `docs/contract/rules.md`.

use schemars::JsonSchema;
use serde::Serialize;
use slopvac_engine::CORE_PACK;
use slopvac_model::{
    AiSignal, CategoryId, GenreId, NonEmptyString, PackId, RuleId, SchemaVersion, Severity,
    x_extensions,
};
use slopvac_rules::{
    AiSignalSource, AppliesTo, ApplyMode, BlockKind, Case, Citation, ContextGate, Example,
    Exception, LoadedCategory, LoadedRule, Match, MatchKind, Scope, TextTypeSchema, UniqueVec,
    Weight, judgement_constraints,
};

use crate::rule_catalog::Catalog;
use crate::severity::Effective;

/// The document that `slopvac rules --format json` prints.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(rename = "RulesOutput", transform = x_extensions)]
pub struct RulesOutput<'a> {
    schema_version: SchemaVersion,
    /// The first plugin in `plugins`, or `core` when the list is empty.
    plugin: PackId,
    /// The plugins that own the listed rules, in catalogue order, each once.
    #[schemars(extend("uniqueItems" = true))]
    plugins: Vec<PackId>,
    /// Every category the catalogue loads, in catalogue order. A filter on the rules does not
    /// shorten this list.
    categories: Vec<CategoryOutput<'a>>,
    /// The rules that pass the filters, in catalogue order.
    rules: Vec<RuleOutput<'a>>,
}

impl<'a> RulesOutput<'a> {
    /// The listing of `rules` over the categories of `set`.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "`core` is a valid plugin id, as the rule catalogue also relies on"
    )]
    pub fn new(set: &'a Catalog, rules: Vec<RuleOutput<'a>>) -> Self {
        let mut plugins: Vec<PackId> = Vec::new();
        for rule in &rules {
            if !plugins.contains(&rule.plugin) {
                plugins.push(rule.plugin.clone());
            }
        }
        let plugin = plugins
            .first()
            .cloned()
            .unwrap_or_else(|| PackId::new(CORE_PACK).expect("core is a valid plugin id"));
        let categories = set
            .sets()
            .iter()
            .flat_map(|plugin| {
                plugin
                    .categories()
                    .iter()
                    .map(move |category| CategoryOutput::new(plugin.pack(), category))
            })
            .collect();
        Self {
            schema_version: SchemaVersion,
            plugin,
            plugins,
            categories,
            rules,
        }
    }

    /// The document as pretty-printed JSON with a final newline.
    #[must_use]
    pub fn render(&self) -> String {
        render(self)
    }
}

/// A category as the listing shows it: the metadata of a category file without its rules and
/// without `schema_version`.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(rename = "CategoryOutput", transform = x_extensions)]
pub struct CategoryOutput<'a> {
    id: &'a CategoryId,
    /// The plugin that ships the category.
    plugin: &'a PackId,
    /// Display title.
    title: &'a NonEmptyString,
    /// What the category covers.
    description: &'a NonEmptyString,
    /// Contribution to the overall score before any config override.
    weight: Weight,
    /// Genres this category suits.
    #[schemars(extend("uniqueItems" = true))]
    recommended_for: &'a [GenreId],
}

impl<'a> CategoryOutput<'a> {
    fn new(plugin: &'a PackId, category: &'a LoadedCategory) -> Self {
        Self {
            id: category.id(),
            plugin,
            title: category.title(),
            description: category.description(),
            weight: category.weight(),
            recommended_for: category.recommended_for(),
        }
    }
}

/// The kind of the rule's match, as `--kind` names it. It repeats `match.kind`.
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(rename = "MatchKind", extend("x-slopvac-extensible" = true))]
enum RuleKind {
    Tokens,
    Pattern,
    Substitution,
    Metric,
    Structure,
    Capitalization,
    Vocabulary,
    Judgement,
}

impl From<MatchKind> for RuleKind {
    fn from(kind: MatchKind) -> Self {
        match kind {
            MatchKind::Tokens => Self::Tokens,
            MatchKind::Pattern => Self::Pattern,
            MatchKind::Substitution => Self::Substitution,
            MatchKind::Metric => Self::Metric,
            MatchKind::Structure => Self::Structure,
            MatchKind::Capitalization => Self::Capitalization,
            MatchKind::Vocabulary => Self::Vocabulary,
            MatchKind::Judgement => Self::Judgement,
        }
    }
}

/// The level a rule runs at after the configuration layers. The set is closed, because gate
/// and score logic depend on these values. `off` means the configuration disables the rule,
/// which a shipped rule never does.
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(rename = "EffectiveSeverity")]
enum EffectiveSeverity {
    Suggestion,
    Warning,
    Error,
    Off,
}

impl EffectiveSeverity {
    /// The level named by `Effective::level`, which holds one of the four words.
    fn of(effective: &Effective) -> Self {
        match effective.level.as_str() {
            "suggestion" => Self::Suggestion,
            "warning" => Self::Warning,
            "error" => Self::Error,
            "off" => Self::Off,
            other => unreachable!("severity {other:?} is not a level"),
        }
    }
}

/// One entry of the `rules` list: the rule as its category file defines it, with the
/// qualified id, and the facts the binary adds.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(
    rename = "RuleOutput",
    transform = x_extensions,
    extend("allOf" = [judgement_constraints()])
)]
pub struct RuleOutput<'a> {
    id: &'a RuleId,
    /// Short imperative label.
    name: &'a NonEmptyString,
    /// Shown on a finding. Names the fix, not the violation. {match} and {replacement}
    /// interpolate.
    message: &'a NonEmptyString,
    /// The level the rule ships with.
    severity: Severity,
    scope: Scope,
    #[schemars(with = "TextTypeSchema")]
    text_type: &'a slopvac_text::TextType,
    #[serde(rename = "match")]
    match_: &'a Match,
    #[serde(skip_serializing_if = "Option::is_none")]
    exclude_blocks: Option<&'a UniqueVec<BlockKind>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contexts: Option<&'a ContextGate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    applies_to: Option<&'a AppliesTo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exceptions: Option<&'a UniqueVec<Exception>>,
    /// Literal strings that never fire, even when the match holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    allowlist: Option<&'a UniqueVec<NonEmptyString>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    examples: Option<&'a Vec<Example>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cases: Option<&'a Vec<Case>>,
    /// The rewrite operation, as guidance.
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<&'a String>,
    /// How strongly a finding indicates machine-written text.
    ai_signal: AiSignal,
    /// Where the `ai_signal` value comes from.
    ai_signal_source: AiSignalSource,
    citation: &'a Citation,
    /// The category that holds the rule.
    category: &'a CategoryId,
    /// The plugin that supplies the rule. It is the part of `id` before the colon.
    plugin: PackId,
    kind: RuleKind,
    effective_severity: EffectiveSeverity,
    /// The configuration source that set the effective severity, such as `project
    /// slopvac.toml:4:1`. Present when the configuration set it, including to the level the
    /// rule ships with.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    severity_from: Option<String>,
    /// The documentation page of the rule. Absent when the rule has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    url: Option<String>,
}

impl<'a> RuleOutput<'a> {
    /// The entry of `rule`, shown at `effective` severity with the page `url`.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "a rule id carries a valid plugin id, because RuleId::new checks it"
    )]
    pub fn new(rule: &'a LoadedRule, effective: &Effective, url: Option<String>) -> Self {
        let definition = rule.rule();
        Self {
            id: rule.id(),
            name: &definition.name,
            message: &definition.message,
            severity: definition.severity,
            scope: definition.scope,
            text_type: &definition.text_type,
            match_: &definition.match_,
            exclude_blocks: definition.exclude_blocks.as_ref(),
            contexts: definition.contexts.as_ref(),
            applies_to: definition.applies_to.as_ref(),
            exceptions: definition.exceptions.as_ref(),
            allowlist: definition.allowlist.as_ref(),
            examples: definition.examples.as_ref(),
            cases: definition.cases.as_ref(),
            fix: definition.fix.as_ref(),
            ai_signal: definition.ai_signal,
            ai_signal_source: definition.ai_signal_source,
            citation: &definition.citation,
            category: rule.category(),
            plugin: PackId::new(rule.id().pack()).expect("a rule id names a plugin"),
            kind: rule.kind().into(),
            effective_severity: EffectiveSeverity::of(effective),
            severity_from: effective.from.clone(),
            url,
        }
    }
}

/// The document that `slopvac explain RULE --format json` prints: a rule entry with
/// `schema_version` and, when the rule has an exception, the comment that suppresses it.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(rename = "ExplainOutput", transform = x_extensions)]
pub struct ExplainOutput<'a> {
    #[serde(flatten)]
    rule: RuleOutput<'a>,
    schema_version: SchemaVersion,
    /// The `slopvac-allow` comment that suppresses the rule, naming the rule and its first
    /// exception.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    suppression: Option<String>,
}

impl<'a> ExplainOutput<'a> {
    /// The document for `rule`, with the comment that suppresses it, when it has one.
    #[must_use]
    pub fn new(rule: RuleOutput<'a>, suppression: Option<String>) -> Self {
        Self {
            rule,
            schema_version: SchemaVersion,
            suppression,
        }
    }

    /// The document as pretty-printed JSON with a final newline.
    #[must_use]
    pub fn render(&self) -> String {
        render(self)
    }
}

fn render(document: &impl Serialize) -> String {
    let mut text = serde_json::to_string_pretty(document).unwrap_or_default();
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::KINDS;

    #[test]
    fn a_kind_prints_the_name_that_the_kind_option_takes() {
        for (name, kind) in KINDS {
            let printed = serde_json::to_string(&RuleKind::from(kind)).unwrap();
            assert_eq!(printed, format!("\"{name}\""));
        }
    }
}
