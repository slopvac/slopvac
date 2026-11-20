//! Validated scalar values. Each constructor enforces the constraint the published
//! `lint-report` schema states, and `Deserialize` goes through the same constructor.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::ids::{IdError, is_kebab, is_pack};

/// A number or text that breaks its constraint.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {kind}: {value}")]
pub struct ValueError {
    kind: &'static str,
    value: String,
}

/// Findings per 100 words: a finite number, never negative.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Density(f64);

impl Density {
    /// Zero.
    pub const ZERO: Self = Self(0.0);

    /// Wrap a density, rejecting NaN, infinity and negatives.
    pub fn new(value: f64) -> Result<Self, ValueError> {
        if value.is_finite() && value >= 0.0 {
            Ok(Self(value))
        } else {
            Err(ValueError {
                kind: "density",
                value: value.to_string(),
            })
        }
    }

    /// The number.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Density {
    type Error = ValueError;
    fn try_from(value: f64) -> Result<Self, ValueError> {
        Self::new(value)
    }
}

impl From<Density> for f64 {
    fn from(value: Density) -> Self {
        value.0
    }
}

impl JsonSchema for Density {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Density".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "number", "minimum": 0 })
    }
}

/// Quality score from 0 (worst) to 100: a finite number in that range.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Score(f64);

impl Score {
    /// The best score.
    pub const BEST: Self = Self(100.0);

    /// Wrap a score, rejecting NaN, infinity and values outside 0 to 100.
    pub fn new(value: f64) -> Result<Self, ValueError> {
        if value.is_finite() && (0.0..=100.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ValueError {
                kind: "score",
                value: value.to_string(),
            })
        }
    }

    /// The number.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Score {
    type Error = ValueError;
    fn try_from(value: f64) -> Result<Self, ValueError> {
        Self::new(value)
    }
}

impl From<Score> for f64 {
    fn from(value: Score) -> Self {
        value.0
    }
}

impl JsonSchema for Score {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Score".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "number", "minimum": 0, "maximum": 100 })
    }
}

/// Text that is never empty.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NonEmptyString(String);

impl NonEmptyString {
    /// Wrap text, rejecting the empty string.
    pub fn new(value: impl Into<String>) -> Result<Self, ValueError> {
        let value = value.into();
        if value.is_empty() {
            Err(ValueError {
                kind: "text",
                value: "empty string".into(),
            })
        } else {
            Ok(Self(value))
        }
    }

    /// The text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NonEmptyString {
    type Error = ValueError;
    fn try_from(value: String) -> Result<Self, ValueError> {
        Self::new(value)
    }
}

impl From<NonEmptyString> for String {
    fn from(value: NonEmptyString) -> Self {
        value.0
    }
}

impl fmt::Display for NonEmptyString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl JsonSchema for NonEmptyString {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "NonEmptyString".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "minLength": 1 })
    }
}

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident, $kind:literal, $check:path, $pattern:literal) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap the text.
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if $check(&value) {
                    Ok(Self(value))
                } else {
                    Err(IdError::new($kind, value))
                }
            }

            /// The text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;
            fn try_from(value: String) -> Result<Self, IdError> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({ "type": "string", "pattern": $pattern })
            }
        }
    };
}

id_newtype!(
    /// Category identifier: `^[a-z][a-z0-9-]*(?![\s\S])`.
    CategoryId, "category id", is_pack, "^[a-z][a-z0-9-]*(?![\\s\\S])"
);
id_newtype!(
    /// Genre identifier: `^[a-z][a-z0-9-]*(?![\s\S])`.
    GenreId, "genre id", is_pack, "^[a-z][a-z0-9-]*(?![\\s\\S])"
);
id_newtype!(
    /// Stable kebab-case name, such as a notice code or genre signal:
    /// `^[a-z][a-z0-9]*(-[a-z0-9]+)*(?![\s\S])`.
    KebabName, "kebab-case name", is_kebab, "^[a-z][a-z0-9]*(-[a-z0-9]+)*(?![\\s\\S])"
);

/// Read an optional member that is either absent or a real value, never `null`.
///
/// Use with `#[serde(default, deserialize_with = "present")]` so `"x": null` is an error
/// instead of reading as absent.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}
/// Read a member that must be present but may be `null` (a missing member is an error).
pub(crate) fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
