use thiserror::Error;

use crate::messages;

/// Why a style could not be loaded or resolved.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StyleError {
    /// No style has this name.
    #[error("{}", messages::not_found(.0))]
    NotFound(String),
    /// The name is not a plain style name.
    #[error("{}", messages::invalid_name(.0))]
    InvalidName(String),
    /// The file is not valid TOML or does not match the shape.
    #[error("{}", messages::parse(style, message))]
    Parse {
        /// Style name.
        style: String,
        /// Parser message with position.
        message: String,
    },
    /// The file breaks the style schema: an unknown key, a wrong value, or a
    /// `schema_version` other than 3.
    #[error("{}", messages::invalid(style, rendered))]
    Invalid {
        /// Style name.
        style: String,
        /// `(code, JSON pointer)` of each problem, such as `("config-unknown-key", "/foo")`.
        problems: Vec<(String, String)>,
        /// The problems rendered with source snippets.
        rendered: String,
    },
    /// `extends` forms a cycle.
    #[error("{}", messages::cycle(.0))]
    Cycle(Vec<String>),
    /// Two active styles declare a conflict and strict mode is on.
    #[error("{}", messages::conflict(loser, winner))]
    Conflict {
        /// Earlier style.
        loser: String,
        /// Later style.
        winner: String,
    },
    /// The file is larger than [`MAX_STYLE_BYTES`](crate::MAX_STYLE_BYTES).
    #[error("{}", messages::too_large(style, *cap))]
    TooLarge {
        /// Style name.
        style: String,
        /// The cap that the file exceeded, in bytes.
        cap: u64,
    },
    /// Reading a style file failed.
    #[error("{}", messages::io(style, message))]
    Io {
        /// Style name.
        style: String,
        /// OS message.
        message: String,
    },
}
