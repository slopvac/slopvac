//! Context taggers, the tag registry and genres.

mod builtin;
mod checklist;
mod cli;
mod declarative;
mod declared;
mod genre;
mod mixed;
mod pattern;
mod reference;
mod schema;
mod structural;
mod tag;
mod view;

pub use builtin::{
    EnforcedInvariantTagger, ListLeadInTagger, RequirementTagger, builtin_genres, builtin_registry,
};
pub use checklist::{ChecklistEntry, ChecklistRule, checklist, effective_gates, rule_allows};
pub use cli::{command, render, run};
pub use declarative::{DeclarativeTagger, Matcher, TaggerSpec, Unit};
pub use declared::{DeclaredContextTagger, context_hint};
pub use genre::{
    Budget, CategorySettings, CategoryTable, Detect, GenreAssignment, GenreRegistry, GenreResult,
    GenreSource, GenreSpec, NoBudget, Profile, Reason, ResolveOptions, RuleLevel, Scalar, Signal,
    SignalKind, files_match, parse_genres,
};
pub use mixed::{MIXED_TYPES_RULE_ID, mixed_types_entry, mixed_types_rule};
pub use pattern::{Pattern, normalize_path};
pub use reference::reference_pages;
pub use structural::{FrontMatterCondition, Scope, When};
pub use tag::{ContextGate, Tag, TagIndex, TagRegistry, Tagger};
pub use view::{
    BlockKind, BlockView, DocumentView, OwnedBlock, PlainDocument, SentenceView, split_sentences,
};

pub use pattern::MAX_PATTERN_BYTES;

/// Errors from this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A regular expression failed to compile.
    #[error("invalid pattern `{pattern}`: {source}")]
    Pattern {
        /// Pattern source.
        pattern: String,
        /// The PCRE2 compile error.
        #[source]
        source: slopvac_pcre2::CompileError,
    },
    /// A pattern is longer than the data-pack limit.
    #[error("pattern is {len} bytes, over the {max}-byte limit")]
    PatternTooLong {
        /// Pattern length in bytes.
        len: usize,
        /// The limit, [`MAX_PATTERN_BYTES`].
        max: usize,
    },
    /// PCRE2 failed while matching, for example on a match or JIT stack limit.
    #[error("pattern `{pattern}` failed while matching: {source}")]
    Match {
        /// Pattern source.
        pattern: String,
        /// The PCRE2 match error.
        #[source]
        source: slopvac_pcre2::MatchError,
    },
    /// A tagger id was registered twice.
    #[error("tagger `{0}` is already registered")]
    DuplicateTagger(String),
    /// A genre id that no definition provides.
    #[error("unknown genre `{0}`")]
    UnknownGenre(String),
    /// A YAML data file did not parse.
    #[error("{0}")]
    Yaml(#[source] serde_saphyr::Error),
    /// A `files` pattern is not a valid gitignore-style glob.
    #[error("glob `{pattern}`: {source}")]
    Glob {
        /// The pattern as written.
        pattern: String,
        /// The glob parser error.
        #[source]
        source: ignore::Error,
    },
    /// A list of `files` patterns did not compile as a set.
    #[error("{0}")]
    GlobSet(#[source] ignore::Error),
    /// A `path` signal glob of a genre is invalid.
    #[error("genre `{genre}`: glob `{glob}`: {source}")]
    SignalGlob {
        /// Genre id.
        genre: String,
        /// The glob as written.
        glob: String,
        /// The glob parser error.
        #[source]
        source: globset::Error,
    },
    /// A file could not be read. The path and the OS message are escaped for a terminal.
    #[error("{}: {}", slopvac_sanitize::line(.path), slopvac_sanitize::line(&.source.to_string()))]
    Read {
        /// The path as given.
        path: String,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// A data file did not parse or validate.
    #[error("{0}")]
    Parse(String),
}
