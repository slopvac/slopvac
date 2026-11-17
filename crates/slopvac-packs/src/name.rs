//! Qualified names: `<pack>:<name>`.

use std::fmt;
use std::str::FromStr;

use slopvac_model::PackId;

/// A name that is not valid. The text is raw; `Display` escapes it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    /// The pack part is not a pack id.
    #[error("`{}` is not a pack id", slopvac_sanitize::line(.0))]
    Pack(String),
    /// The local part is empty, too long, starts with something other than a letter or digit,
    /// or holds a character outside `[A-Za-z0-9._-]`.
    #[error(
        "`{}` is not a name (letters, digits, `.`, `_` and `-`, starting with a letter or digit)",
        slopvac_sanitize::line(.0)
    )]
    Local(String),
}

/// Longest name part, in bytes, so a hostile value cannot flood a message.
const MAX_NAME_BYTES: usize = 128;

pub(crate) fn is_local(s: &str) -> bool {
    let mut chars = s.chars();
    s.len() <= MAX_NAME_BYTES
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A pack id and a name inside that pack, written `<pack>:<name>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QualifiedName {
    pack: PackId,
    name: String,
}

impl QualifiedName {
    /// Build a qualified name, checking the local part.
    pub fn new(pack: PackId, name: impl Into<String>) -> Result<Self, NameError> {
        let name = name.into();
        if is_local(&name) {
            Ok(Self { pack, name })
        } else {
            Err(NameError::Local(name))
        }
    }

    /// The owning pack.
    #[must_use]
    pub fn pack(&self) -> &PackId {
        &self.pack
    }

    /// The name inside the pack.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl FromStr for QualifiedName {
    type Err = NameError;

    fn from_str(s: &str) -> Result<Self, NameError> {
        let (pack, name) = s
            .split_once(':')
            .ok_or_else(|| NameError::Pack(s.to_owned()))?;
        let pack = PackId::new(pack).map_err(|_| NameError::Pack(pack.to_owned()))?;
        Self::new(pack, name)
    }
}

impl fmt::Display for QualifiedName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.pack, self.name)
    }
}
