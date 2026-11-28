//! The levels a rule or category reports at, declared once.

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, Visitor};

use crate::messages::ConfigPhrase;

/// The level a rule or category reports at. This enum is the one declaration of the level
/// names. The readers, the style loader and the `RuleLevel` definition in the schemas all take
/// their spelling from [`Level::as_str`], and [`Level::ALL`] is the ordered list of every level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl Level {
    /// Every level, in the order the schemas and the reference pages list them.
    pub const ALL: [Self; 4] = [Self::Error, Self::Warning, Self::Suggestion, Self::Off];

    /// The spelling of the level in a config file, a style file and the schemas.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Suggestion => "suggestion",
            Self::Off => "off",
        }
    }
}

/// The spellings of [`Level::ALL`], in the same order.
pub(crate) const NAMES: &[&str] = &names();

const fn names() -> [&'static str; Level::ALL.len()] {
    let mut out = [""; Level::ALL.len()];
    let mut i = 0;
    while i < Level::ALL.len() {
        out[i] = Level::ALL[i].as_str();
        i += 1;
    }
    out
}

impl<'de> Deserialize<'de> for Level {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Spelling;

        impl Visitor<'_> for Spelling {
            type Value = Level;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(ConfigPhrase::KindLevelName.as_static())
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Level, E> {
                Level::ALL
                    .into_iter()
                    .find(|level| level.as_str() == value)
                    .ok_or_else(|| E::unknown_variant(value, NAMES))
            }
        }

        deserializer.deserialize_str(Spelling)
    }
}
