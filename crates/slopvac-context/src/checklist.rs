//! Review checklist filtering by context.
//!
//! The judgement remainder of a core rule inherits the core rule's `contexts` gate, and a
//! checklist drops rules that cannot apply to the contexts a document has. Rule data comes
//! from the rule loader; this module sees only ids and gates.

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

/// One kept checklist entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistEntry {
    /// Rule id.
    pub id: String,
    /// Hints for the contexts that occur in the document, in context order.
    pub hints: Vec<String>,
}

/// The gate a rule runs under: its own gate plus the gate of its core rule. A remainder is
/// exempt wherever its core rule is exempt, and runs only where both allow it.
#[must_use]
pub fn effective_gate(rule: &ChecklistRule, rules: &[ChecklistRule]) -> ContextGate {
    let mut gate = rule.gate.clone();
    if let Some(core) = rule
        .core
        .as_ref()
        .and_then(|id| rules.iter().find(|r| &r.id == id))
    {
        for s in &core.gate.skip {
            if !gate.skip.contains(s) {
                gate.skip.push(s.clone());
            }
        }
        if gate.only.is_empty() {
            gate.only.clone_from(&core.gate.only);
        } else if !core.gate.only.is_empty() {
            gate.only.retain(|o| core.gate.only.contains(o));
        }
    }
    gate
}

/// Build the checklist for one document.
///
/// `seen` lists the context kinds that occur in the document. `covering` lists kinds that
/// cover all of its prose, such as `change-note` on a changelog. A rule drops out when its
/// effective gate needs a kind that does not occur, or skips a kind that covers everything.
#[must_use]
pub fn checklist(rules: &[ChecklistRule], seen: &[&str], covering: &[&str]) -> Vec<ChecklistEntry> {
    let mut out = Vec::new();
    for rule in rules {
        let gate = effective_gate(rule, rules);
        let needs_missing =
            !gate.only.is_empty() && !gate.only.iter().any(|o| seen.contains(&o.as_str()));
        let skipped_everywhere = gate.skip.iter().any(|s| covering.contains(&s.as_str()));
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
