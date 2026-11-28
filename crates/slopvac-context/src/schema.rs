//! Pieces shared by the pack file structs (`schemas/v3/genre.schema.json` and
//! `tagger.schema.json`): the `x-` extension rule and the bounds. The name checks are
//! `slopvac_config::{is_name, is_ref, is_rule_key}`.

use std::fmt;

use serde::Deserialize;
use serde::de::{self, IgnoredAny, MapAccess, Visitor};

use slopvac_model::SchemaVersion;

use crate::Error;

/// The only `schema_version` of a genre file this crate reads: the one `slopvac-model` holds
/// in `SchemaVersion::VALUE`, which the schema states too.
pub const GENRE_SCHEMA_VERSION: u32 = SchemaVersion::VALUE as u32;

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

/// A signal weight and the `min_score` of a genre must be greater than this. The reader checks
/// it and the schema states it as `exclusiveMinimum`.
pub(crate) const EXCLUSIVE_MINIMUM: u32 = 0;

/// The weight of a signal that sets none.
pub(crate) const DEFAULT_WEIGHT: u32 = 1;

/// The score a genre needs to be a candidate when its `detect` sets none.
pub(crate) const DEFAULT_MIN_SCORE: u32 = 1;

/// The fewest signals of a `detect` block.
pub(crate) const MIN_SIGNALS: usize = 1;

#[cfg(test)]
mod patterns;
