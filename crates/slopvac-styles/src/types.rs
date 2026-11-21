//! The value types of a style preset, as `schemas/v3/style.schema.json` defines them.
//!
//! Merging is the job of the layer stack in `slopvac-config`: maps merge by key and lists
//! replace.

use serde::Deserialize;

use crate::messages::StyleMessage;

/// The level a rule or category reports at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Fails the run.
    Error,
    /// Reported, does not fail on its own.
    Warning,
    /// Informational.
    Suggestion,
    /// Disabled.
    Off,
}

/// Per-category dials. A bare level stands for `{ severity = <level> }`.
#[derive(Debug, Clone, PartialEq)]
pub enum CategoryPatch {
    /// A bare level.
    Level(Level),
    /// The table form.
    Dials(CategoryDials),
}

impl<'de> Deserialize<'de> for CategoryPatch {
    // A hand-written visitor keeps the specific error of the branch that
    // matched; an untagged enum would replace it with "did not match any variant".
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{IntoDeserializer, MapAccess, Visitor, value::MapAccessDeserializer};

        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = CategoryPatch;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a level or a table of category dials")
            }

            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<CategoryPatch, E> {
                Level::deserialize(s.into_deserializer()).map(CategoryPatch::Level)
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<CategoryPatch, A::Error> {
                CategoryDials::deserialize(MapAccessDeserializer::new(map))
                    .map(CategoryPatch::Dials)
            }
        }
        d.deserialize_any(V)
    }
}

/// The table form of a category entry.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryDials {
    /// Level the category reports at.
    pub severity: Option<Level>,
    /// Lowest level that inherited rules may report at. `off` means no floor
    /// and cancels a floor set by a lower layer.
    pub minimum_severity: Option<Level>,
    /// Density budget. `"none"` removes a budget set by a lower layer.
    pub max_per_100_words: Option<Budget>,
    /// Score weight.
    pub weight: Option<f64>,
}

/// A density budget: a limit, or `"none"` to clear the limit of a lower layer.
///
/// `Some(Budget::None)` is a value, not a missing field, so it wins over a
/// lower layer's limit when patches merge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Budget {
    /// At most this many findings per 100 words.
    Limit(f64),
    /// No budget.
    None,
}

impl<'de> Deserialize<'de> for Budget {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Number(f64),
            Text(String),
        }
        match Raw::deserialize(d)? {
            Raw::Number(n) if n >= 0.0 => Ok(Self::Limit(n)),
            Raw::Number(n) => Err(D::Error::custom(StyleMessage::BudgetNegative {
                found: n.to_string(),
            })),
            Raw::Text(t) if t == "none" => Ok(Self::None),
            Raw::Text(t) => Err(D::Error::custom(StyleMessage::BudgetInvalid { found: t })),
        }
    }
}

/// The `terms` table of a style.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Terms {
    /// Words the style lets through the spelling check. A list replaces.
    pub allow: Option<Vec<String>>,
    /// Blocklist files, relative to the style file. A list replaces.
    pub blocklists: Option<Vec<String>>,
}

impl CategoryPatch {
    /// The entry in table form: a bare level becomes `{ severity = <level> }`.
    #[must_use]
    pub fn dials(&self) -> CategoryDials {
        match self {
            Self::Level(l) => CategoryDials {
                severity: Some(*l),
                ..CategoryDials::default()
            },
            Self::Dials(d) => d.clone(),
        }
    }
}
