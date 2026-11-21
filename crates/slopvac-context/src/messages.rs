//! The messages `slopvac-context` reports, as one catalog that the binary registers through
//! [`message_entries`].
//!
//! [`ContextMessage`] holds the wording of each [`crate::Error`] variant. `Error` fills the
//! text parameters with escaped or raw values and renders the entry.

use slopvac_messages::{Catalog, Entry};

/// The wording of each [`crate::Error`] variant.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum ContextMessage {
    /// A regular expression failed to compile.
    #[code("context-pattern-invalid")]
    #[text("invalid pattern `{pattern}`: {reason}")]
    PatternInvalid { pattern: String, reason: String },
    /// A tagger pattern is longer than the pattern cap.
    #[code("tagger-pattern-too-long")]
    #[message("pattern is {len} bytes, over the {max}-byte limit")]
    PatternTooLong {
        #[count]
        len: usize,
        #[count]
        max: usize,
    },
    /// A pattern failed while matching, for example on a match or JIT stack limit.
    #[code("context-match-failed")]
    #[text("pattern `{pattern}` failed while matching: {reason}")]
    MatchFailed { pattern: String, reason: String },
    /// A tagger id was registered twice.
    #[code("context-tagger-duplicate")]
    #[text("tagger `{id}` is already registered")]
    TaggerDuplicate { id: String },
    /// The taggers produced more tags than the document is allowed.
    #[code("context-tag-limit")]
    #[text("the document has more than {limit} context tags")]
    TagLimit {
        #[count]
        limit: usize,
    },
    /// A genre id that no definition provides.
    #[code("context-genre-unknown")]
    #[text("unknown genre `{id}`")]
    GenreUnknown { id: String },
    /// A YAML data file did not parse. The reason is the parser's own message.
    #[code("context-yaml-invalid")]
    #[text("{reason}")]
    YamlInvalid { reason: String },
    /// A `files` pattern is not a valid gitignore-style glob.
    #[code("context-glob-invalid")]
    #[text("glob `{glob}`: {reason}")]
    GlobInvalid { glob: String, reason: String },
    /// A list of `files` patterns did not compile as a set. The reason is the parser's own
    /// message.
    #[code("context-glob-set-invalid")]
    #[text("{reason}")]
    GlobSetInvalid { reason: String },
    /// A `path` signal glob of a genre is invalid.
    #[code("context-signal-glob-invalid")]
    #[text("genre `{genre}`: glob `{glob}`: {reason}")]
    SignalGlobInvalid {
        genre: String,
        glob: String,
        reason: String,
    },
    /// A file could not be read.
    #[code("context-read-failed")]
    #[text("{path}: {reason}")]
    ReadFailed {
        #[display_path]
        path: String,
        reason: String,
    },
    /// A file is larger than the document size cap.
    #[code("context-file-too-large")]
    #[text("{path}: larger than the {cap}-byte document limit")]
    FileTooLarge {
        #[display_path]
        path: String,
        #[count]
        cap: u64,
    },
    /// A file was refused without a read.
    #[code("context-file-unreadable")]
    #[text("{path}: cannot read")]
    FileUnreadable {
        #[display_path]
        path: String,
    },
    /// A data file did not parse or validate. The reason is the validator's own message.
    #[code("context-data-invalid")]
    #[text("{reason}")]
    DataInvalid { reason: String },
}

/// The entries of the context catalog.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    ContextMessage::entries()
}
