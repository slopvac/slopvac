use thiserror::Error;

/// Why a style could not be loaded or resolved.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StyleError {
    /// No style has this name.
    #[error("unknown style `{0}`")]
    NotFound(String),
    /// The name is not a plain style name.
    #[error("invalid style name `{0}`")]
    InvalidName(String),
    /// The file is not valid TOML or does not match the shape.
    #[error("style `{style}`: {message}")]
    Parse {
        /// Style name.
        style: String,
        /// Parser message with position.
        message: String,
    },
    /// The file has a key the shape does not define.
    #[error("style `{style}`: unknown key `{key}`")]
    UnknownKey {
        /// Style name.
        style: String,
        /// Offending key.
        key: String,
    },
    /// The file declares an unsupported `schema_version`.
    #[error("style `{style}`: schema_version {found} is not supported, expected 3")]
    SchemaVersion {
        /// Style name.
        style: String,
        /// Version found.
        found: u32,
    },
    /// `extends` forms a cycle.
    #[error("style extends cycle: {}", .0.join(" -> "))]
    Cycle(Vec<String>),
    /// Two active styles declare a conflict and strict mode is on.
    #[error("style `{loser}` conflicts with `{winner}`; `{winner}` wins")]
    Conflict {
        /// Earlier style.
        loser: String,
        /// Later style.
        winner: String,
    },
    /// Reading a style file failed.
    #[error("reading style `{style}`: {message}")]
    Io {
        /// Style name.
        style: String,
        /// OS message.
        message: String,
    },
}
