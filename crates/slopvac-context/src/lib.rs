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

/// Errors from this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A regular expression failed to compile.
    #[error("invalid pattern `{pattern}`: {message}")]
    Pattern {
        /// Pattern source.
        pattern: String,
        /// PCRE2 message.
        message: String,
    },
    /// PCRE2 failed while matching, for example on a match or JIT stack limit.
    #[error("pattern `{pattern}` failed while matching: {message}")]
    Match {
        /// Pattern source.
        pattern: String,
        /// PCRE2 message.
        message: String,
    },
    /// A tagger id was registered twice.
    #[error("tagger `{0}` is already registered")]
    DuplicateTagger(String),
    /// A genre id that no definition provides.
    #[error("unknown genre `{0}`")]
    UnknownGenre(String),
    /// A data file did not parse or validate.
    #[error("{0}")]
    Parse(String),
}
