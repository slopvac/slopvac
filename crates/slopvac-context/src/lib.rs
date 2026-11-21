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
mod work;

pub use builtin::{
    BuiltinFile, EnforcedInvariantTagger, GENRE_FILES, ListLeadInTagger, RequirementTagger,
    TAGGER_FILES, builtin_genres, builtin_registry,
};
pub use checklist::{
    ChecklistEntry, ChecklistRule, checklist, checklist_rules, effective_gates, rule_allows,
};
pub use cli::{command, render, run};
pub use declarative::{DeclarativeTagger, Matcher, TaggerSpec, Unit};
pub use declared::{DeclaredContextTagger, context_hint};
pub use genre::{
    Budget, CategorySettings, CategoryTable, DeclaredGenre, Detect, GenreAssignment, GenreRegistry,
    GenreResult, GenreSource, GenreSpec, NoBudget, Profile, Reason, ResolveOptions, RuleLevel,
    Scalar, Signal, SignalKind, parse_genres,
};
pub use mixed::mixed_types_entry;
pub use pattern::{Pattern, normalize_path};
pub use reference::reference_pages;
pub use schema::{GENRE_SCHEMA_VERSION, TAGGER_SCHEMA_VERSION};
pub use structural::{FrontMatterCondition, Scope, When};
pub use tag::{ContextGate, Tag, TagIndex, TagRegistry, TagSink, Tagger, normalize_tags};
pub use view::{
    BlockKind, BlockView, DocumentView, OwnedBlock, PlainDocument, SentenceView, split_sentences,
};
#[cfg(feature = "work-counters")]
#[doc(hidden)]
pub use work::{
    take_admonition_steps, take_build_elements, take_heading_steps, take_lookup_steps,
    take_materialised_tags, take_negation_steps, take_section_steps, take_text_steps,
};

pub use pattern::MAX_PATTERN_BYTES;

/// Errors from this crate. Text from documents, configs and plugins reaches `Display` only
/// through `slopvac_sanitize::line`; the fields and sources stay raw. [`Error::raw_message`]
/// is the same message before that escaping, for a caller that escapes it where it writes it.
///
/// `Display` is written by hand, not derived, because it must sanitize text and share its
/// wording with `raw_message` (the exception in `docs/engineering.md`, Errors).
#[derive(Debug)]
pub enum Error {
    /// A regular expression failed to compile.
    Pattern {
        /// Pattern source.
        pattern: String,
        /// The PCRE2 compile error.
        source: slopvac_pcre2::CompileError,
    },
    /// A pattern is longer than the data-pack limit.
    PatternTooLong {
        /// Pattern length in bytes.
        len: usize,
        /// The limit, [`MAX_PATTERN_BYTES`].
        max: usize,
    },
    /// PCRE2 failed while matching, for example on a match or JIT stack limit.
    Match {
        /// Pattern source.
        pattern: String,
        /// The PCRE2 match error.
        source: slopvac_pcre2::MatchError,
    },
    /// A tagger id was registered twice.
    DuplicateTagger(String),
    /// The taggers produced more tags than the document is allowed, so no index was built.
    TagLimit {
        /// The most tags the document was allowed.
        limit: usize,
    },
    /// A genre id that no definition provides.
    UnknownGenre(String),
    /// A YAML data file did not parse.
    Yaml(serde_saphyr::Error),
    /// A `files` pattern is not a valid gitignore-style glob.
    Glob {
        /// The pattern as written.
        pattern: String,
        /// The glob parser error.
        source: ignore::Error,
    },
    /// A list of `files` patterns did not compile as a set.
    GlobSet(ignore::Error),
    /// A `path` signal glob of a genre is invalid.
    SignalGlob {
        /// Genre id.
        genre: String,
        /// The glob as written.
        glob: String,
        /// The glob parser error.
        source: globset::Error,
    },
    /// A file could not be read. The path and the OS message are escaped for a terminal.
    Read {
        /// The path as given.
        path: String,
        /// The OS error.
        source: std::io::Error,
    },
    /// A file is larger than the document size cap, so none of it was read.
    TooLarge {
        /// The path as given.
        path: String,
        /// The cap in bytes, `slopvac_fs::MAX_DOCUMENT_BYTES`.
        cap: u64,
    },
    /// A file was refused without a read: it is not a regular file, or it could not be opened
    /// safely.
    Unreadable {
        /// The path as given.
        path: String,
    },
    /// A data file did not parse or validate.
    Parse(String),
}

impl Error {
    /// The message of this error with its untrusted parts (patterns, ids, paths, OS and
    /// parser messages) left unescaped. `Display` is this text, escaped.
    ///
    /// The caller must escape the result once where it writes it (a terminal line or a
    /// report). It must never reach a log, a panic or an error chain.
    #[must_use]
    pub fn raw_message(&self) -> String {
        self.render(str::to_owned)
    }

    /// The one wording of every message; `escape` is applied to each untrusted part.
    fn render(&self, escape: fn(&str) -> String) -> String {
        match self {
            Self::Pattern { pattern, source } => format!(
                "invalid pattern `{}`: {}",
                escape(pattern),
                escape(&source.to_string())
            ),
            Self::PatternTooLong { len, max } => {
                format!("pattern is {len} bytes, over the {max}-byte limit")
            }
            Self::Match { pattern, source } => format!(
                "pattern `{}` failed while matching: {}",
                escape(pattern),
                escape(&source.to_string())
            ),
            Self::DuplicateTagger(id) => format!("tagger `{}` is already registered", escape(id)),
            Self::TagLimit { limit } => format!("the document has more than {limit} context tags"),
            Self::UnknownGenre(id) => format!("unknown genre `{}`", escape(id)),
            Self::Yaml(source) => escape(&source.to_string()),
            Self::Glob { pattern, source } => format!(
                "glob `{}`: {}",
                escape(pattern),
                escape(&source.to_string())
            ),
            Self::GlobSet(source) => escape(&source.to_string()),
            Self::SignalGlob {
                genre,
                glob,
                source,
            } => format!(
                "genre `{}`: glob `{}`: {}",
                escape(genre),
                escape(glob),
                escape(&source.to_string())
            ),
            Self::Read { path, source } => {
                format!("{}: {}", escape(path), escape(&source.to_string()))
            }
            Self::TooLarge { path, cap } => {
                format!(
                    "{}: larger than the {cap}-byte document limit",
                    escape(path)
                )
            }
            Self::Unreadable { path } => format!("{}: cannot read", escape(path)),
            Self::Parse(text) => escape(text),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render(slopvac_sanitize::line))
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pattern { source, .. } => Some(source),
            Self::Match { source, .. } => Some(source),
            Self::Yaml(source) => Some(source),
            Self::Glob { source, .. } | Self::GlobSet(source) => Some(source),
            Self::SignalGlob { source, .. } => Some(source),
            Self::Read { source, .. } => Some(source),
            Self::PatternTooLong { .. }
            | Self::DuplicateTagger(_)
            | Self::TagLimit { .. }
            | Self::UnknownGenre(_)
            | Self::TooLarge { .. }
            | Self::Unreadable { .. }
            | Self::Parse(_) => None,
        }
    }
}
