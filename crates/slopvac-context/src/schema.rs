//! Pieces shared by the pack file structs (`schemas/v3/genre.schema.json` and
//! `tagger.schema.json`): the `x-` extension rule and the name patterns.

use std::fmt;

use serde::Deserialize;
use serde::de::{self, IgnoredAny, MapAccess, Visitor};

use crate::Error;

/// The only `schema_version` of a genre file this crate reads.
pub const GENRE_SCHEMA_VERSION: u32 = 3;

/// The only `schema_version` of a declarative tagger file this crate reads.
pub const TAGGER_SCHEMA_VERSION: u32 = 3;

/// Keys that no field claims. Keys with the prefix `x-` are ignored; any other key is an
/// error, as in the schemas (`patternProperties` `^x-` with `additionalProperties: false`).
#[derive(Debug, Clone, Default)]
pub(crate) struct Extensions;

impl<'de> Deserialize<'de> for Extensions {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Extensions;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("only known keys and keys that start with `x-`")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Extensions, A::Error> {
                while let Some(key) = map.next_key::<String>()? {
                    if !key.starts_with("x-") {
                        return Err(de::Error::custom(format!("unknown field `{key}`")));
                    }
                    map.next_value::<IgnoredAny>()?;
                }
                Ok(Extensions)
            }
        }
        d.deserialize_map(V)
    }
}

/// `deserialize_with` for optional fields: absent is `None`, but a present `null` is an
/// error, because the schemas give every field a type that excludes null.
pub(crate) fn present<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

pub(crate) fn check_schema_version(what: &str, v: u32, expected: u32) -> Result<(), Error> {
    if v == expected {
        Ok(())
    } else {
        Err(Error::Parse(format!(
            "{what}: schema_version is {v}, expected {expected}"
        )))
    }
}

/// `^[a-z][a-z0-9-]*$`: genre ids, tagger ids and `ContextTag`.
pub(crate) fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `Ref`: `^([a-z][a-z0-9-]*:)?[a-z][a-z0-9-]*$`.
pub(crate) fn is_ref(s: &str) -> bool {
    match s.split_once(':') {
        Some((pack, name)) => is_name(pack) && is_name(name),
        None => is_name(s),
    }
}

/// `RuleKey`: `^([a-z][a-z0-9-]*:)?[A-Za-z0-9][A-Za-z0-9._-]*$`.
pub(crate) fn is_rule_key(s: &str) -> bool {
    let name = match s.split_once(':') {
        Some((pack, name)) if is_name(pack) => name,
        Some(_) => return false,
        None => s,
    };
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}
