//! Built-in taggers: declarative ones from `taggers.yaml` (text) and `structural.yaml`
//! (`when`), and sentence-level code taggers.

mod leadin;
mod sentence;
mod text;

use crate::{Error, GenreRegistry, TagRegistry, TaggerSpec, parse_genres};

pub use leadin::ListLeadInTagger;
pub use sentence::{EnforcedInvariantTagger, RequirementTagger};

const TAGGERS: &str = include_str!("builtin/taggers.yaml");
const STRUCTURAL: &str = include_str!("builtin/structural.yaml");
const GENRES: &str = include_str!("builtin/genres.yaml");

/// The built-in genres. `consumer` is the fallback.
pub fn builtin_genres() -> Result<GenreRegistry, Error> {
    GenreRegistry::new(parse_genres(GENRES)?, "consumer")
}

/// A registry with every built-in tagger.
pub fn builtin_registry() -> Result<TagRegistry, Error> {
    let mut reg = TagRegistry::new();
    for spec in TaggerSpec::list_from_yaml(STRUCTURAL)? {
        reg.register(Box::new(spec.compile()?))?;
    }
    for spec in TaggerSpec::list_from_yaml(TAGGERS)? {
        reg.register(Box::new(spec.compile()?))?;
    }
    reg.register(Box::new(ListLeadInTagger::new()?))?;
    reg.register(Box::new(RequirementTagger::new()?))?;
    reg.register(Box::new(EnforcedInvariantTagger::new()?))?;
    Ok(reg)
}
