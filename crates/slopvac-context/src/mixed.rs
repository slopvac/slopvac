//! The mixed-type judgement item: a page that reaches two or more Diataxis content types.
//!
//! Detection comes from [`GenreRegistry::mixed_types`]. The item rides the checklist hook
//! as a [`ChecklistEntry`] whose hint names the types, so a reviewer sees which ones to split.

use crate::{ChecklistEntry, ChecklistRule, ContextGate, DocumentView, Error, GenreRegistry};

/// Genre whose descendants are the content types.
const CONTENT_TYPE_PARENT: &str = "consumer";

/// STUB(slopvac-8l4.5.15): crate-local id until the rule YAML defines the rule.
pub const MIXED_TYPES_RULE_ID: &str = "prose-craft.mixed-content-types";

/// STUB(slopvac-8l4.5.15): crate-local definition of the judgement rule. It has no context
/// gate and no core rule; it fires from document structure, not from a context tag.
#[must_use]
pub fn mixed_types_rule() -> ChecklistRule {
    ChecklistRule {
        id: MIXED_TYPES_RULE_ID.into(),
        gate: ContextGate::default(),
        core: None,
        hint_contexts: Vec::new(),
    }
}

/// The judgement item for `doc`, or `None` when the page reaches at most one content type.
/// The entry's single hint names the types, best match first.
pub fn mixed_types_entry(
    genres: &GenreRegistry,
    doc: &dyn DocumentView,
) -> Result<Option<ChecklistEntry>, Error> {
    let found = genres.mixed_types(doc, CONTENT_TYPE_PARENT)?;
    if found.len() < 2 {
        return Ok(None);
    }
    let names: Vec<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
    Ok(Some(ChecklistEntry {
        id: mixed_types_rule().id,
        hints: vec![format!(
            "This page mixes content types ({}). Split it so each page serves one reader need.",
            names.join(", ")
        )],
    }))
}
