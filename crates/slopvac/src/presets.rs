//! Style presets for the layer stack: the presets that ship inside the binary.
//!
//! STUB(slopvac-8l4.4.10): genres and `<pack>:<name>` config targets come from packs, and the
//! pack loader has not landed, so [`BuiltinPresets`] supplies styles only. Blocklist files in
//! a style are not read yet either.

use serde_json::{Value, json};
use slopvac_config::{Layer, PackFile, Presets, Setting};
use slopvac_styles::{
    Budget, BuiltinSource, CategoryPatch, Level, SettingsPatch, StyleError, resolve,
};

/// Styles bundled with the binary, as a config [`Presets`].
#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinPresets;

/// The names of the bundled styles.
pub fn style_names() -> Vec<&'static str> {
    BuiltinSource::names().collect()
}

fn level(l: Level) -> Value {
    json!(match l {
        Level::Error => "error",
        Level::Warning => "warning",
        Level::Suggestion => "suggestion",
        Level::Off => "off",
    })
}

fn settings_of(patch: &SettingsPatch) -> Vec<Setting> {
    let set = |path: &[&str], value: Value| Setting::bare(Layer::Style, path, value);
    let mut out = Vec::new();
    for (rule, l) in &patch.rules {
        out.push(set(&["rules", rule], level(*l)));
    }
    for (category, patch) in &patch.categories {
        let dials = match patch {
            CategoryPatch::Level(l) => {
                out.push(set(&["categories", category, "severity"], level(*l)));
                continue;
            }
            CategoryPatch::Dials(d) => d,
        };
        let at = |key: &'static str| ["categories", category.as_str(), key];
        if let Some(l) = dials.severity {
            out.push(set(&at("severity"), level(l)));
        }
        if let Some(l) = dials.minimum_severity {
            out.push(set(&at("minimum_severity"), level(l)));
        }
        if let Some(b) = dials.max_per_100_words {
            let value = match b {
                Budget::Limit(n) => json!(n),
                Budget::None => json!("none"),
            };
            out.push(set(&at("max_per_100_words"), value));
        }
        if let Some(w) = dials.weight {
            out.push(set(&at("weight"), json!(w)));
        }
    }
    if let Some(allow) = &patch.terms.allow {
        out.push(set(&["terms", "allow"], json!(allow)));
    }
    out
}

impl BuiltinPresets {
    /// Whether a style of this name ships with the binary.
    #[must_use]
    pub fn has(name: &str) -> bool {
        BuiltinSource::names().any(|n| n == name)
    }

    /// The settings of a bundled style, with the styles it extends applied first.
    ///
    /// # Errors
    /// A malformed bundled style.
    pub fn settings(name: &str) -> Result<Vec<Setting>, StyleError> {
        let resolution = resolve(&[name.to_owned()], &BuiltinSource, false)?;
        Ok(settings_of(&resolution.patch))
    }
}

impl Presets for BuiltinPresets {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        Self::settings(name).ok()
    }

    fn genre_chain(&self, _name: &str) -> Vec<Vec<Setting>> {
        Vec::new()
    }

    fn pack_config(&self, _name: &str) -> Option<PackFile> {
        None
    }
}
