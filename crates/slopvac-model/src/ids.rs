use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::schema;

/// A malformed identifier.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {kind} {value:?}")]
pub struct IdError {
    kind: &'static str,
    value: String,
}

pub(crate) fn is_pack(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `^[a-z][a-z0-9]*(-[a-z0-9]+)*$`.
pub(crate) fn is_kebab(s: &str) -> bool {
    let mut parts = s.split('-');
    let first_ok = parts.next().is_some_and(|p| {
        p.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    });
    first_ok
        && parts.all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

fn is_local(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl IdError {
    pub(crate) fn new(kind: &'static str, value: String) -> Self {
        Self { kind, value }
    }
}

/// Pack identifier: the namespace of a rule. Built-in rules live in the reserved pack core.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String", extend("pattern" = schema::SLUG))]
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

/// Qualified rule identifier <pack>:<id>. A rule from a category file has the id
/// <category>.<rule>, for example core:prose-craft.acronym-periods; a rule a plugin reports
/// may use a bare id such as core:passive-voice. Short names resolve in configuration only
/// where they are unambiguous; every output carries the qualified form.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String", extend("pattern" = schema::RULE_ID))]
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

/// Finding severity. Excludes off: 2.x config accepts off to disable a rule, so off belongs
/// to config, not to findings.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Lowest severity.
    Suggestion,
    /// Gating warning.
    Warning,
    /// Gating error.
    Error,
}

/// Text-context label such as prose or code.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String", extend("pattern" = schema::SLUG))]
pub struct ContextTag(String);

impl ContextTag {
    /// Validate and wrap a context tag.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if is_pack(&value) {
            Ok(Self(value))
        } else {
            Err(IdError {
                kind: "context tag",
                value,
            })
        }
    }

    /// The tag as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContextTag {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, IdError> {
        Self::new(value)
    }
}

impl From<ContextTag> for String {
    fn from(tag: ContextTag) -> Self {
        tag.0
    }
}

/// The root artifact `schema_version`. It is always 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SchemaVersion;

impl Serialize for SchemaVersion {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(3)
    }
}

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let n = u64::deserialize(deserializer)?;
        if n == 3 {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(format!(
                "schema_version must be 3, got {n}"
            )))
        }
    }
}

impl JsonSchema for SchemaVersion {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SchemaVersion".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "Root artifact schema version.",
            "const": 3,
        })
    }
}

/// Plugin API version, separate from schema_version.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "u32", into = "u32")]
#[schemars(with = "u32", extend("minimum" = 1))]
pub struct ApiVersion(u32);

impl ApiVersion {
    /// Wrap a version, rejecting zero.
    pub fn new(value: u32) -> Result<Self, IdError> {
        if value >= 1 {
            Ok(Self(value))
        } else {
            Err(IdError {
                kind: "api version",
                value: value.to_string(),
            })
        }
    }

    /// The number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for ApiVersion {
    type Error = IdError;
    fn try_from(value: u32) -> Result<Self, IdError> {
        Self::new(value)
    }
}

impl From<ApiVersion> for u32 {
    fn from(v: ApiVersion) -> Self {
        v.0
    }
}
/// Text contexts a match or document sits in. Each tag appears once.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "Vec<ContextTag>", into = "Vec<ContextTag>")]
pub struct Contexts(Vec<ContextTag>);

/// A context tag listed twice.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("context tag {0:?} is listed twice")]
pub struct DuplicateContext(String);

impl Contexts {
    /// Wrap a tag list, rejecting duplicates. Order is kept.
    pub fn new(tags: Vec<ContextTag>) -> Result<Self, DuplicateContext> {
        for (i, tag) in tags.iter().enumerate() {
            if tags[..i].contains(tag) {
                return Err(DuplicateContext(tag.as_str().to_owned()));
            }
        }
        Ok(Self(tags))
    }

    /// The tags in order.
    #[must_use]
    pub fn as_slice(&self) -> &[ContextTag] {
        &self.0
    }
}

impl std::ops::Deref for Contexts {
    type Target = [ContextTag];
    fn deref(&self) -> &[ContextTag] {
        &self.0
    }
}

impl TryFrom<Vec<ContextTag>> for Contexts {
    type Error = DuplicateContext;
    fn try_from(tags: Vec<ContextTag>) -> Result<Self, DuplicateContext> {
        Self::new(tags)
    }
}

impl From<Contexts> for Vec<ContextTag> {
    fn from(contexts: Contexts) -> Self {
        contexts.0
    }
}

impl JsonSchema for Contexts {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Contexts".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let item = generator.subschema_for::<ContextTag>();
        schemars::json_schema!({ "type": "array", "items": item, "uniqueItems": true })
    }
}
