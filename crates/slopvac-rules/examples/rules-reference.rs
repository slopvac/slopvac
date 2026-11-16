//! Write the rule reference pages under a directory.
//!
//! `cargo run -p slopvac-rules --example rules-reference -- <out-dir>` writes `rules/index.md`,
//! one index per category and one page per rule below `<out-dir>`.
//!
//! STUB(slopvac-8l4.19.15): load the shipped core pack once it exists. Until then the
//! example loads the v3 conversion of the 2.x rules, the data the CLI embeds in
//! `crates/slopvac/src/rule_catalog.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, PresetLevels, RuleSet, reference_pages};
use slopvac_styles::{BuiltinSource, resolve};

/// The string values one level below `key` in the merged settings: rule levels by rule key,
/// or category severities by category key.
fn levels(settings: &Value, key: &str, field: Option<&str>) -> BTreeMap<String, String> {
    let Some(map) = settings.get(key).and_then(Value::as_object) else {
        return BTreeMap::new();
    };
    map.iter()
        .filter_map(|(name, value)| {
            let level = match field {
                Some(f) => value.get(f),
                None => Some(value),
            }?;
            Some((name.clone(), level.as_str()?.to_owned()))
        })
        .collect()
}

/// The levels each bundled style preset sets, with its `extends` chain applied.
fn presets() -> Vec<PresetLevels> {
    BuiltinSource::names()
        .map(|name| {
            let settings = resolve(&[name.to_owned()], &BuiltinSource, false)
                .expect("bundled presets resolve")
                .resolved
                .to_json();
            PresetLevels {
                name: name.to_owned(),
                categories: levels(&settings, "categories", Some("severity")),
                rules: levels(&settings, "rules", None),
            }
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).expect("usage: reference <out-dir>"));
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/core-v3");
    let set = RuleSet::load_dir(&PackId::new("core")?, &dir, &CompileOptions::default())
        .map_err(|e| e.to_string())?;
    for (path, text) in reference_pages(&set, &presets()) {
        let file = out.join(path);
        std::fs::create_dir_all(file.parent().expect("page paths have a directory"))?;
        std::fs::write(file, text)?;
    }
    Ok(())
}
