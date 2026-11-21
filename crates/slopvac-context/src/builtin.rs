//! Built-in genres and taggers. Each genre and each declarative tagger is one file under
//! `builtin/genres/` or `builtin/taggers/`, as `genre.schema.json` and `tagger.schema.json`
//! define them. Patterns are PCRE2 and case-insensitive; path globs ignore case. A genre needs
//! a score of 3 to be detected. The code taggers are in the submodules.

mod leadin;
mod sentence;
mod text;

use crate::{Error, GenreRegistry, GenreSpec, TagRegistry, TaggerSpec};

pub use leadin::ListLeadInTagger;
pub use sentence::{EnforcedInvariantTagger, RequirementTagger};

/// One built-in artifact file: its path in the core pack and its text.
pub type BuiltinFile = (&'static str, &'static str);

/// Build a table of the YAML files under `builtin/<dir>/`, keyed `<dir>/<name>.yaml`.
macro_rules! yaml_files {
    ($dir:literal: $($name:literal),+ $(,)?) => {
        &[$((
            concat!($dir, "/", $name, ".yaml"),
            include_str!(concat!("builtin/", $dir, "/", $name, ".yaml")),
        )),+]
    };
}

/// The built-in genre files in registration order, one genre per file. The registration order
/// is the tie-break order of detection, so keep it.
pub const GENRE_FILES: &[BuiltinFile] = yaml_files!("genres":
    "consumer", "reference", "change-comms", "internal", "informal", "security", "ui-copy",
    "agent-instructions", "tutorial", "how-to", "reference-page", "explanation",
);

/// The built-in declarative tagger files in registration order, one tagger per file. The
/// structural taggers (`when`) come first, then the text tagger (`match`).
pub const TAGGER_FILES: &[BuiltinFile] = yaml_files!("taggers":
    "change-note-heading", "change-note-changelog-heading", "change-note-version-heading",
    "change-note-admonition", "change-note-file", "procedure-heading", "safety-admonition",
    "safety-heading", "example-heading", "example-admonition", "quote-block",
    "generated-heading", "generated-frontmatter", "generated-notice",
    "change-note-version-sentence",
);

/// The built-in genres, which belong to the `core` pack, so `core:consumer` resolves. `consumer`
/// is the fallback.
pub fn builtin_genres() -> Result<GenreRegistry, Error> {
    let genres = GENRE_FILES
        .iter()
        .map(|(_, text)| GenreSpec::from_yaml(text))
        .collect::<Result<Vec<_>, _>>()?;
    GenreRegistry::new(genres, "consumer")?.in_pack("core")
}

/// A registry with every built-in tagger.
pub fn builtin_registry() -> Result<TagRegistry, Error> {
    let mut reg = TagRegistry::new();
    for (_, text) in TAGGER_FILES {
        reg.register(Box::new(TaggerSpec::from_yaml(text)?.compile()?))?;
    }
    reg.register(Box::new(ListLeadInTagger::new()?))?;
    reg.register(Box::new(RequirementTagger::new()?))?;
    reg.register(Box::new(EnforcedInvariantTagger::new()?))?;
    Ok(reg)
}
