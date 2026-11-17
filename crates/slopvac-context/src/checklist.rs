//! Review checklist filtering by context.
//!
//! The judgement remainder of a core rule inherits the core rule's `contexts` gate, and a
//! checklist drops rules that cannot apply to the contexts a document has. Rule data comes
//! from the rule loader; this module sees only ids and gates.

use slopvac_rules::{LoadedRule, MatchKind, RuleSet};

use crate::{ContextGate, context_hint};

/// A rule as the checklist sees it.
#[derive(Debug, Clone)]
pub struct ChecklistRule {
    /// Qualified rule id.
    pub id: String,
    /// The rule's own `contexts` gate.
    pub gate: ContextGate,
    /// For a judgement remainder, the id of the core rule it completes.
    pub core: Option<String>,
    /// Contexts whose hint belongs in this rule's entry when the context occurs.
    pub hint_contexts: Vec<String>,
}

impl ChecklistRule {
    /// The checklist view of a loaded rule: its qualified id and its `contexts` gate.
    /// A loaded rule names no core rule and no hint contexts, so both stay empty.
    #[must_use]
    pub fn from_loaded(rule: &LoadedRule) -> Self {
        let gate = rule
            .rule()
            .contexts
            .as_ref()
            .map_or_else(ContextGate::default, |g| {
                let names = |tags: &Option<slopvac_rules::UniqueVec<slopvac_model::ContextTag>>| {
                    tags.as_ref().map_or_else(Vec::new, |t| {
                        t.as_slice().iter().map(|k| k.as_str().to_owned()).collect()
                    })
                };
                ContextGate {
                    only: names(&g.only),
                    skip: names(&g.skip),
                }
            });
        Self {
            id: rule.id().as_str().to_owned(),
            gate,
            core: None,
            hint_contexts: Vec::new(),
        }
    }
}

/// The judgement rules of `set` as checklist rules, in load order.
#[must_use]
pub fn checklist_rules(set: &RuleSet) -> Vec<ChecklistRule> {
    set.rules_of_kind(MatchKind::Judgement)
        .map(ChecklistRule::from_loaded)
        .collect()
}

/// One kept checklist entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistEntry {
    /// Rule id.
    pub id: String,
    /// Hints for the contexts that occur in the document, in context order.
    pub hints: Vec<String>,
}

/// The gates a rule runs under: its own gate, then the gate of its core rule. A gate is an
/// `only` OR-list plus a `skip` list, so the two cannot merge into one gate without
/// changing meaning. A remainder runs where every gate allows it, which makes it exempt
/// wherever its core rule is exempt.
#[must_use]
pub fn effective_gates(rule: &ChecklistRule, rules: &[ChecklistRule]) -> Vec<ContextGate> {
    let mut gates = vec![rule.gate.clone()];
    if let Some(core) = rule
        .core
        .as_ref()
        .and_then(|id| rules.iter().find(|r| &r.id == id))
    {
        gates.push(core.gate.clone());
    }
    gates
}

/// Whether a rule may report on text that carries `kinds`: every effective gate allows it.
#[must_use]
pub fn rule_allows(rule: &ChecklistRule, rules: &[ChecklistRule], kinds: &[&str]) -> bool {
    effective_gates(rule, rules).iter().all(|g| g.allows(kinds))
}

/// Build the checklist for one document.
///
/// `seen` lists the context kinds that occur in the document. `covering` lists kinds that
/// cover all of its prose, such as `change-note` on a changelog. A rule drops out when some
/// effective gate needs a kind that does not occur, or skips a kind that covers everything.
#[must_use]
pub fn checklist(rules: &[ChecklistRule], seen: &[&str], covering: &[&str]) -> Vec<ChecklistEntry> {
    let mut out = Vec::new();
    for rule in rules {
        let gates = effective_gates(rule, rules);
        let needs_missing = gates
            .iter()
            .any(|g| !g.only.is_empty() && !g.only.iter().any(|o| seen.contains(&o.as_str())));
        let skipped_everywhere = gates
            .iter()
            .any(|g| g.skip.iter().any(|s| covering.contains(&s.as_str())));
        if needs_missing || skipped_everywhere {
            continue;
        }
        let hints = rule
            .hint_contexts
            .iter()
            .filter(|k| seen.contains(&k.as_str()))
            .filter_map(|k| context_hint(k))
            .map(String::from)
            .collect();
        out.push(ChecklistEntry {
            id: rule.id.clone(),
            hints,
        });
    }
    out
}
