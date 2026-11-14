use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A malformed identifier.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {kind} {value:?}")]
pub struct IdError {
    kind: &'static str,
    value: String,
}

fn is_pack(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_local(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Pack identifier: `^[a-z][a-z0-9-]*$`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PackId(String);

impl PackId {
    /// Validate and wrap a pack id.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if is_pack(&value) {
            Ok(Self(value))
        } else {
            Err(IdError {
                kind: "pack id",
                value,
            })
        }
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PackId {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, IdError> {
        Self::new(value)
    }
}

impl From<PackId> for String {
    fn from(id: PackId) -> Self {
        id.0
    }
}

impl fmt::Display for PackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Qualified rule identifier `<pack>:<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RuleId(String);

impl RuleId {
    /// Validate and wrap a qualified rule id.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        match value.split_once(':') {
            Some((pack, local)) if is_pack(pack) && is_local(local) => Ok(Self(value)),
            _ => Err(IdError {
                kind: "rule id",
                value,
            }),
        }
    }

    /// The full `<pack>:<id>` text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The pack part.
    #[must_use]
    pub fn pack(&self) -> &str {
        self.0.split_once(':').map_or("", |(p, _)| p)
    }

    /// The part after the colon.
    #[must_use]
    pub fn local(&self) -> &str {
        self.0.split_once(':').map_or("", |(_, l)| l)
    }
}

impl TryFrom<String> for RuleId {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, IdError> {
        Self::new(value)
    }
}

impl From<RuleId> for String {
    fn from(id: RuleId) -> Self {
        id.0
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Finding severity. `off` belongs to config, not to findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Lowest severity.
    Suggestion,
    /// Gating warning.
    Warning,
    /// Gating error.
    Error,
}
