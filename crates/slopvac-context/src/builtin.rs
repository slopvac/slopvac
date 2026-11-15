//! Built-in taggers: declarative ones from `taggers.yaml`, sentence-level code taggers.

mod leadin;
mod sentence;
mod text;

use crate::{Error, GenreRegistry, TagRegistry, TaggerSpec, parse_genres};

pub use leadin::ListLeadInTagger;
pub use sentence::{EnforcedInvariantTagger, RequirementTagger};

const SPECS: &str = include_str!("builtin/taggers.yaml");
const GENRES: &str = include_str!("builtin/genres.yaml");

/// The built-in genres. `consumer` is the fallback.
pub fn builtin_genres() -> Result<GenreRegistry, Error> {
    GenreRegistry::new(parse_genres(GENRES)?, "consumer")
}

/// A registry with every built-in tagger.
pub fn builtin_registry() -> Result<TagRegistry, Error> {
    let specs: Vec<TaggerSpec> =
        serde_saphyr::from_str(SPECS).map_err(|e| Error::Parse(e.to_string()))?;
    let mut reg = TagRegistry::new();
    for spec in specs {
        reg.register(Box::new(spec.compile()?))?;
    }
    reg.register(Box::new(ListLeadInTagger::new()?))?;
    reg.register(Box::new(RequirementTagger::new()?))?;
    reg.register(Box::new(EnforcedInvariantTagger::new()?))?;
    Ok(reg)
}
