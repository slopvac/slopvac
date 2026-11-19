use thiserror::Error;

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
            Self::MissingWord => "has no word",
            Self::UnknownPos => "has an unrecognized part of speech",
            Self::MissingReason => "has no reason",
            Self::Duplicate => "is a duplicate",
            Self::ConflictingCasing => "sets both casing modes",
            Self::BadKind => "has a kind that is not a kebab-case label",
            Self::BadVariant => "has an empty or repeated variant",
            Self::Other => "is not valid",
        }
    }
}

/// Why a terms or blocklist file could not be loaded.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TermsError {
    /// The file could not be read.
    #[error("cannot read {path}: {message}")]
    Io {
        /// File path as given.
        path: String,
        /// OS message.
        message: String,
    },
    /// The file is larger than the cap. Its stable code is `terms-file-too-large`, or
    /// `document-too-large` for a document.
    #[error("[{code}] {path} is larger than {cap} bytes")]
    TooLarge {
        /// The stable code.
        code: &'static str,
        /// File path as given.
        path: String,
        /// The cap that the file exceeded, in bytes.
        cap: u64,
    },
    /// The file is not valid for its format.
    #[error("cannot parse {path}: {message}")]
    Parse {
        /// File path as given.
        path: String,
        /// Parser message with position.
        message: String,
    },
    /// A blocklist has no `schema_version`, as a 2.x blocklist.
    #[error(
        "{path}: missing `schema_version`; a 3.0 blocklist sets `schema_version = 3`. \
         Run `slopvac migrate` to move a 2.x blocklist to 3.0"
    )]
    MissingSchemaVersion {
        /// File path as given.
        path: String,
    },
    /// The file declares an unsupported `schema_version`.
    #[error(
        "{path}: schema_version {found} is not supported, expected the integer 3. \
         Run `slopvac migrate` to move an older file to 3.0"
    )]
    SchemaVersion {
        /// File path as given.
        path: String,
        /// Value found, as JSON text: `2`, `"3"` or `3.5`.
        found: String,
    },
    /// The file has an unknown key.
    #[error("{path}: unknown key `{key}`")]
    UnknownKey {
        /// File path as given.
        path: String,
        /// Offending key.
        key: String,
    },
    /// A term or blocklist entry is invalid.
    #[error("{path}: {message}")]
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
