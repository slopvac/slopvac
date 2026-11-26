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
    /// A text holds more sentences than its size allows.
    #[code("context-sentence-limit")]
    #[text("the text has more than {limit} sentences")]
    SentenceLimit {
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
    /// A pattern budget stopped a compile before it finished.
    #[code("context-pattern-budget-stopped")]
    #[text("a pattern budget stopped the compile")]
    PatternBudgetStopped,
}

/// The entries of the context catalog.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    ContextMessage::entries()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use slopvac_messages::Kind;

    use super::*;

    #[rstest]
    #[case::pattern_invalid(
        ContextMessage::PatternInvalid { pattern: "(".to_owned(), reason: "missing )".to_owned() },
        "invalid pattern `(`: missing )"
    )]
    #[case::pattern_too_long(
        ContextMessage::PatternTooLong { len: 9000, max: 8192 },
        "pattern is 9000 bytes, over the 8192-byte limit"
    )]
    #[case::match_failed(
        ContextMessage::MatchFailed { pattern: "a+".to_owned(), reason: "match limit".to_owned() },
        "pattern `a+` failed while matching: match limit"
    )]
    #[case::tagger_duplicate(
        ContextMessage::TaggerDuplicate { id: "list".to_owned() },
        "tagger `list` is already registered"
    )]
    #[case::tag_limit(
        ContextMessage::TagLimit { limit: 65_536 },
        "the document has more than 65536 context tags"
    )]
    #[case::sentence_limit(
        ContextMessage::SentenceLimit { limit: 65_536 },
        "the text has more than 65536 sentences"
    )]
    #[case::genre_unknown(
        ContextMessage::GenreUnknown { id: "memo".to_owned() },
        "unknown genre `memo`"
    )]
    #[case::yaml_invalid(ContextMessage::YamlInvalid { reason: "bad key".to_owned() }, "bad key")]
    #[case::glob_invalid(
        ContextMessage::GlobInvalid { glob: "[".to_owned(), reason: "unclosed".to_owned() },
        "glob `[`: unclosed"
    )]
    #[case::glob_set_invalid(
        ContextMessage::GlobSetInvalid { reason: "too big".to_owned() },
        "too big"
    )]
    #[case::signal_glob_invalid(
        ContextMessage::SignalGlobInvalid {
            genre: "memo".to_owned(),
            glob: "[".to_owned(),
            reason: "unclosed".to_owned(),
        },
        "genre `memo`: glob `[`: unclosed"
    )]
    #[case::read_failed(
        ContextMessage::ReadFailed { path: "a.md".to_owned(), reason: "denied".to_owned() },
        "a.md: denied"
    )]
    #[case::file_too_large(
        ContextMessage::FileTooLarge { path: "a.md".to_owned(), cap: 7 },
        "a.md: larger than the 7-byte document limit"
    )]
    #[case::file_unreadable(
        ContextMessage::FileUnreadable { path: "a.md".to_owned() },
        "a.md: cannot read"
    )]
    #[case::data_invalid(ContextMessage::DataInvalid { reason: "bad file".to_owned() }, "bad file")]
    #[case::pattern_budget_stopped(
        ContextMessage::PatternBudgetStopped,
        "a pattern budget stopped the compile"
    )]
    fn each_message_renders_its_published_text(
        #[case] message: ContextMessage,
        #[case] expected: &str,
    ) {
        assert_eq!(message.to_string(), expected);
    }

    #[test]
    fn the_catalog_registers_each_code_with_its_kind() {
        let entries: Vec<_> = message_entries().iter().map(|e| (e.code, e.kind)).collect();
        assert_eq!(
            entries,
            [
                ("context-pattern-invalid", Kind::Text),
                ("tagger-pattern-too-long", Kind::Error),
                ("context-match-failed", Kind::Text),
                ("context-tagger-duplicate", Kind::Text),
                ("context-tag-limit", Kind::Text),
                ("context-sentence-limit", Kind::Text),
                ("context-genre-unknown", Kind::Text),
                ("context-yaml-invalid", Kind::Text),
                ("context-glob-invalid", Kind::Text),
                ("context-glob-set-invalid", Kind::Text),
                ("context-signal-glob-invalid", Kind::Text),
                ("context-read-failed", Kind::Text),
                ("context-file-too-large", Kind::Text),
                ("context-file-unreadable", Kind::Text),
                ("context-data-invalid", Kind::Text),
                ("context-pattern-budget-stopped", Kind::Text),
            ]
        );
    }
}
