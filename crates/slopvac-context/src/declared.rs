//! Contexts that a document declares instead of having them inferred.
//!
//! Security context comes only from a genre (chosen by front matter, a `[[genres]]`
//! assignment or the genre's own signals) and never from vocabulary. The detector prototype
//! (`slopvac-8l4.2.14`) tagged 40% of security-doc sentences with a lexical tagger, which is
//! too broad to infer from.

use crate::{DocumentView, Error, GenreRegistry, GenreResult, Tag, Tagger};

/// Tags the whole document with the contexts its genre declares.
#[derive(Debug, Clone)]
pub struct DeclaredContextTagger {
    /// Each context with the genre that declares it.
    contexts: Vec<(String, String)>,
}

impl DeclaredContextTagger {
    /// Build the tagger for a resolved genre. `None` when the genre declares no context.
    pub fn for_genre(registry: &GenreRegistry, genre: &GenreResult) -> Result<Option<Self>, Error> {
        Self::from_ids(registry, &genre.id, genre.assigned.as_deref())
    }

    /// Like [`Self::for_genre`], for a caller that holds the genre id and the assigned genre.
    pub fn for_genre_parts(
        registry: &GenreRegistry,
        id: &str,
        assigned: Option<&str>,
    ) -> Result<Option<Self>, Error> {
        Self::from_ids(registry, id, assigned)
    }

    fn from_ids(
        registry: &GenreRegistry,
        id: &str,
        assigned: Option<&str>,
    ) -> Result<Option<Self>, Error> {
        let mut contexts: Vec<(String, String)> = registry
            .contexts(id)?
            .into_iter()
            .map(|c| (c, id.to_owned()))
            .collect();
        // Contexts from a path or config assignment are additive: a front matter genre
        // changes the genre but never removes them.
        if let Some(assigned) = assigned {
            for c in registry.contexts(assigned)? {
                if !contexts.iter().any(|(known, _)| *known == c) {
                    contexts.push((c, assigned.to_owned()));
                }
            }
        }
        if contexts.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self { contexts }))
    }
}

impl Tagger for DeclaredContextTagger {
    fn id(&self) -> &'static str {
        "declared-contexts"
    }

    fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
        Ok(self
            .contexts
            .iter()
            .map(|(kind, genre)| Tag {
                kind: kind.clone(),
                span: 0..doc.source_len(),
                confidence: 1.0,
                evidence: format!("declared by genre `{genre}`").into(),
            })
            .collect())
    }
}

/// Reviewer hint for findings in a context, or `None`. The review checklist and the
/// judgement remainder print it next to the rule.
#[must_use]
pub fn context_hint(kind: &str) -> Option<&'static str> {
    match kind {
        "security" => Some(
            "Security context: a negative guarantee needs the mechanism that enforces it. \
             Name the enforcement, or drop the claim.",
        ),
        _ => None,
    }
}
