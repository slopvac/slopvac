//! Building blocks the schema spells out and `serde` derives do not: extension keys,
//! unique lists, validated text and absent-but-never-null members.

use std::borrow::Cow;
use std::fmt;
use std::ops::Deref;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The `x-` keys an object accepts and ignores. Any other unknown key is an error.
///
/// Embed it as `#[serde(flatten)] #[schemars(skip)]` and add `additionalProperties: false`
/// plus `patternProperties` to the struct's schema.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Extensions;

impl<'de> Deserialize<'de> for Extensions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Extensions;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("only `x-` extension keys")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Extensions, A::Error> {
                while let Some(key) = map.next_key::<String>()? {
                    if !key.starts_with("x-") {
                        return Err(de::Error::custom(format!(
                            "unknown field `{key}` (extension keys start with `x-`)"
                        )));
                    }
                    map.next_value::<IgnoredAny>()?;
                }
                Ok(Extensions)
            }
        }
        deserializer.deserialize_map(V)
    }
}

impl Serialize for Extensions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        serializer.serialize_map(Some(0))?.end()
    }
}

/// A list with at least `MIN` items and no repeats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniqueVec<T, const MIN: usize = 0>(Vec<T>);

impl<T, const MIN: usize> UniqueVec<T, MIN> {
    /// The items in file order.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }
}

impl<T, const MIN: usize> Deref for UniqueVec<T, MIN> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<'a, T, const MIN: usize> IntoIterator for &'a UniqueVec<T, MIN> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'de, T: Deserialize<'de> + PartialEq, const MIN: usize> Deserialize<'de>
    for UniqueVec<T, MIN>
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let items = Vec::<T>::deserialize(deserializer)?;
        if items.len() < MIN {
            return Err(de::Error::custom(format!(
                "expected at least {MIN} item(s), got {}",
                items.len()
            )));
        }
        for (i, item) in items.iter().enumerate() {
            if items[..i].contains(item) {
                return Err(de::Error::custom(format!(
                    "items must be unique, item {i} repeats an earlier one"
                )));
            }
        }
        Ok(Self(items))
    }
}

impl<T: Serialize, const MIN: usize> Serialize for UniqueVec<T, MIN> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<T: JsonSchema, const MIN: usize> JsonSchema for UniqueVec<T, MIN> {
    fn schema_name() -> Cow<'static, str> {
        format!("UniqueArray{MIN}_{}", T::schema_name()).into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = json_schema!({
            "type": "array",
            "items": generator.subschema_for::<T>(),
            "uniqueItems": true,
        });
        if MIN > 0 {
            schema.insert("minItems".into(), MIN.into());
        }
        schema
    }
}

/// Read an optional member that is absent or a real value, never `null`.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// An error for a value that breaks a constraint.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind}: {value:?}")]
pub struct ValueError {
    kind: &'static str,
    value: String,
}

impl ValueError {
    pub(crate) fn new(kind: &'static str, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }
}

/// Define a validated text newtype with a schema `pattern`.
macro_rules! text_newtype {
    ($(#[$meta:meta])* $name:ident, $inline:literal, $description:literal, $kind:literal, $check:expr, $pattern:expr) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap the text.
            pub fn new(value: impl Into<String>) -> Result<Self, $crate::ext::ValueError> {
                let value = value.into();
                let check: fn(&str) -> bool = $check;
                if check(&value) {
                    Ok(Self(value))
                } else {
                    Err($crate::ext::ValueError::new($kind, value))
                }
            }

            /// The text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = $crate::ext::ValueError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn inline_schema() -> bool {
                $inline
            }

            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                let mut schema = schemars::json_schema!({ "type": "string", "pattern": $pattern });
                if !$description.is_empty() {
                    schema.insert("description".into(), $description.into());
                }
                schema
            }
        }
    };
}
pub(crate) use text_newtype;

/// `^[a-z][a-z0-9<separator>]*$`: a lower-case letter, then lower-case letters, digits and the
/// separator.
pub(crate) fn is_slug(s: &str, separator: char) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == separator)
}
