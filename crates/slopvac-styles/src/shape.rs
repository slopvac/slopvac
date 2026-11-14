//! Draft shape of a style preset file.
//!
//! STUB(slopvac-8l4.7.6): the real shape is `schemas/v3/style.schema.json`
//! (slopvac-8l4.2.4). Field names follow the bead text: extends, conflicts,
//! categories, rules, terms.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::patch::SettingsPatch;

/// A style preset as written in a data pack.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Style {
    /// Schema version of the file; must be 3.
    pub schema_version: u32,
    /// One-line description.
    #[serde(default)]
    pub description: String,
    /// Styles resolved before this one, in order.
    #[serde(default)]
    pub extends: Vec<String>,
    /// Styles that this one cannot sit beside without a declared winner.
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// The settings this style sets.
    #[serde(flatten)]
    pub settings: SettingsPatch,
    /// Keys starting with `x-` are ignored; anything else is an error.
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, toml::Value>,
}

impl Style {
    /// Parse a style file. Unknown keys that do not start with `x-` are errors.
    ///
    /// # Errors
    /// Returns [`crate::StyleError::Parse`] for bad TOML or shape, and
    /// [`crate::StyleError::UnknownKey`] or [`crate::StyleError::SchemaVersion`]
    /// for contract violations.
    pub fn parse(name: &str, text: &str) -> Result<Self, crate::StyleError> {
        let style: Self = toml::from_str(text).map_err(|e| crate::StyleError::Parse {
            style: name.to_owned(),
            message: e.to_string(),
        })?;
        if style.schema_version != 3 {
            return Err(crate::StyleError::SchemaVersion {
                style: name.to_owned(),
                found: style.schema_version,
            });
        }
        if let Some(key) = style.extra.keys().find(|k| !k.starts_with("x-")) {
            return Err(crate::StyleError::UnknownKey {
                style: name.to_owned(),
                key: key.clone(),
            });
        }
        Ok(style)
    }
}
