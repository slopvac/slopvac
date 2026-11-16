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
    },
}
