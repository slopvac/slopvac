use thiserror::Error;

use crate::messages::{TermsMessage, TermsPhrase};

/// What is wrong with a term or blocklist entry, without any of the entry's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntryProblem {
    /// A blocklist entry has no word, or a terms entry has an empty term.
    MissingWord,
    /// A blocklist entry has a part of speech that is not recognised.
    UnknownPos,
    /// A blocklist entry has no reason.
    MissingReason,
    /// Two entries name the same word and part of speech, or the same term.
    Duplicate,
    /// A term sets both casing modes.
    ConflictingCasing,
    /// A term has a kind that is not a kebab-case label.
    BadKind,
    /// A term has an empty variant, or lists a variant twice.
    BadVariant,
    /// Any other problem with an entry.
    Other,
}

impl EntryProblem {
    /// A fixed description that holds no user text.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::MissingWord => TermsPhrase::ProblemMissingWord,
            Self::UnknownPos => TermsPhrase::ProblemUnknownPos,
            Self::MissingReason => TermsPhrase::ProblemMissingReason,
            Self::Duplicate => TermsPhrase::ProblemDuplicate,
            Self::ConflictingCasing => TermsPhrase::ProblemConflictingCasing,
            Self::BadKind => TermsPhrase::ProblemBadKind,
            Self::BadVariant => TermsPhrase::ProblemBadVariant,
            Self::Other => TermsPhrase::ProblemOther,
        }
        .as_static()
    }
}

/// Why a terms or blocklist file could not be loaded.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TermsError {
    /// The file could not be read.
    #[error("{}", io_text(.path, .message))]
    Io {
        /// File path as given.
        path: String,
        /// OS message.
        message: String,
    },
    /// The file is larger than the cap. Its stable code is `terms-file-too-large`, or
    /// `document-too-large` for a document.
    #[error("{}", too_large_text(.code, .path, *.cap))]
    TooLarge {
        /// The stable code.
        code: &'static str,
        /// File path as given.
        path: String,
        /// The cap that the file exceeded, in bytes.
        cap: u64,
    },
    /// The file is not valid for its format.
    #[error("{}", parse_text(.path, .message))]
    Parse {
        /// File path as given.
        path: String,
        /// Parser message with position.
        message: String,
    },
    /// A blocklist has no `schema_version`, as a 2.x blocklist.
    #[error("{}", missing_version_text(.path))]
    MissingSchemaVersion {
        /// File path as given.
        path: String,
    },
    /// The file declares an unsupported `schema_version`.
    #[error("{}", schema_version_text(.path, .found))]
    SchemaVersion {
        /// File path as given.
        path: String,
        /// Value found, as JSON text: `2`, `"3"` or `3.5`.
        found: String,
    },
    /// The file has an unknown key.
    #[error("{}", unknown_key_text(.path, .key))]
    UnknownKey {
        /// File path as given.
        path: String,
        /// Offending key.
        key: String,
    },
    /// A term or blocklist entry is invalid.
    #[error("{}", entry_text(.path, .message))]
    Entry {
        /// File path as given.
        path: String,
        /// What is wrong with which entry.
        message: String,
        /// The one-based position of the entry in the file, when known.
        number: Option<usize>,
        /// The kind of problem, free of the entry's text.
        problem: EntryProblem,
    },
}

fn io_text(path: &str, message: &str) -> String {
    TermsMessage::Io {
        path: path.to_owned(),
        message: message.to_owned(),
    }
    .to_string()
}

fn too_large_text(code: &str, path: &str, cap: u64) -> String {
    TermsMessage::TooLarge {
        code: code.to_owned(),
        path: path.to_owned(),
        cap,
    }
    .to_string()
}

fn parse_text(path: &str, message: &str) -> String {
    TermsMessage::Parse {
        path: path.to_owned(),
        message: message.to_owned(),
    }
    .to_string()
}

fn missing_version_text(path: &str) -> String {
    TermsMessage::MissingSchemaVersion {
        path: path.to_owned(),
    }
    .to_string()
}

fn schema_version_text(path: &str, found: &str) -> String {
    TermsMessage::SchemaVersion {
        path: path.to_owned(),
        found: found.to_owned(),
    }
    .to_string()
}

fn unknown_key_text(path: &str, key: &str) -> String {
    TermsMessage::UnknownKey {
        path: path.to_owned(),
        key: key.to_owned(),
    }
    .to_string()
}

fn entry_text(path: &str, message: &str) -> String {
    TermsMessage::Entry {
        path: path.to_owned(),
        message: message.to_owned(),
    }
    .to_string()
}

impl TermsError {
    /// The same error for the entry at one-based position `number`, when it is an entry error
    /// that does not name one yet.
    #[must_use]
    pub fn at_entry(mut self, at: usize) -> Self {
        if let Self::Entry { number, .. } = &mut self
            && number.is_none()
        {
            *number = Some(at);
        }
        self
    }
}
