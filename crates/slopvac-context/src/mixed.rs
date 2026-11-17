//! The mixed-type judgement item: a page that reaches two or more Diataxis content types.
//!
//! Detection comes from [`GenreRegistry::mixed_types`]. The item rides the checklist hook
//! as a [`ChecklistEntry`] whose hint names the types, so a reviewer sees which ones to split.
//! The rule itself, with its id and message, comes from the loaded rule set.

use slopvac_rules::LoadedRule;

use crate::{ChecklistEntry, DocumentView, Error, GenreRegistry};

/// Genre whose descendants are the content types.
const CONTENT_TYPE_PARENT: &str = "consumer";

/// The judgement item for `doc`, or `None` when the page reaches at most one content type.
/// `rule` is the loaded mixed-content-types rule. The entry carries its id, and the single
/// hint is its message followed by the types, best match first.
pub fn mixed_types_entry(
    genres: &GenreRegistry,
    doc: &dyn DocumentView,
    rule: &LoadedRule,
) -> Result<Option<ChecklistEntry>, Error> {
    let found = genres.mixed_types(doc, CONTENT_TYPE_PARENT)?;
    if found.len() < 2 {
        return Ok(None);
    }
    let names: Vec<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
    Ok(Some(ChecklistEntry {
        id: rule.id().as_str().to_owned(),
        hints: vec![format!(
            "{} ({})",
            rule.rule().message.as_str(),
            names.join(", ")
        )],
    }))
}
