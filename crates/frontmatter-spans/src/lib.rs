#![doc = include_str!("../README.md")]

mod de;
mod detect;
mod error;
mod json;
mod span;
mod toml;
mod value;
mod yaml;

use std::fmt;

use serde::Deserialize;

pub use detect::{Block, detect};
pub use error::{DeError, ParseError};
pub use span::{LineCol, LineIndex, Span};
pub use value::{Entry, Node, Value};

/// The syntax of a front-matter block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// Between `---` lines.
    Yaml,
    /// Between `+++` lines.
    Toml,
    /// A JSON object at the start of the document.
    Json,
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Json => "JSON",
        })
    }
}

/// A located and parsed front-matter block.
#[derive(Debug, Clone, PartialEq)]
pub struct FrontMatter<'a> {
    /// Where the block sits.
    pub block: Block,
    /// The raw text between the delimiters. For JSON, the whole object.
    pub raw: &'a str,
    /// The parsed value. Usually a table; YAML can also give a scalar, array or null.
    pub root: Node,
}

impl<'a> FrontMatter<'a> {
    /// Find and parse the front matter at the start of `doc`.
    ///
    /// Returns `Ok(None)` when the document has none.
    pub fn parse(doc: &'a str) -> Result<Option<Self>, ParseError> {
        let Some(block) = detect(doc) else {
            return Ok(None);
        };
        Self::parse_block(doc, block).map(Some)
    }

    /// Parse a block that [`detect`] found in `doc`.
    ///
    /// `block` must come from `detect(doc)` for this same `doc`; otherwise the spans are
    /// meaningless.
    pub fn parse_block(doc: &'a str, block: Block) -> Result<Self, ParseError> {
        let raw = &doc[block.content.clone()];
        let index = LineIndex::new(doc);
        let base = block.content.start;
        let root = match block.format {
            Format::Yaml => yaml::parse(raw, base, &index)?,
            Format::Toml => toml::parse(raw, base, &index)?,
            Format::Json => json::parse(raw, base, &index)?,
        };
        Ok(Self { block, raw, root })
    }

    /// The value for a top-level `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.root.get(key)
    }

    /// Deserialise the whole block into `T`.
    pub fn deserialize<'de, T: Deserialize<'de>>(&'de self) -> Result<T, DeError> {
        self.root.deserialize()
    }

    /// Deserialise the top-level `key` into `T`, with the span of its value.
    ///
    /// Returns `Ok(None)` when the key is absent.
    pub fn deserialize_key<'de, T: Deserialize<'de>>(
        &'de self,
        key: &str,
    ) -> Result<Option<Spanned<T>>, DeError> {
        self.get(key)
            .map(|node| {
                Ok(Spanned {
                    value: node.deserialize()?,
                    span: node.span.clone(),
                })
            })
            .transpose()
    }
}

/// A deserialised value and the span of the source it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    /// The value.
    pub value: T,
    /// Its source range.
    pub span: Span,
}

impl Node {
    /// Deserialise this node into `T`.
    pub fn deserialize<'de, T: Deserialize<'de>>(&'de self) -> Result<T, DeError> {
        T::deserialize(de::NodeDe(self))
    }
}
