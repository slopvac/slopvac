use thiserror::Error;

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
    /// The file is not valid for its format.
    #[error("cannot parse {path}: {message}")]
    Parse {
        /// File path as given.
        path: String,
        /// Parser message with position.
        message: String,
    },
    /// The file declares an unsupported `schema_version`.
    #[error("{path}: schema_version {found} is not supported, expected 3")]
    SchemaVersion {
        /// File path as given.
        path: String,
        /// Version found.
        found: u32,
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
    },
}
