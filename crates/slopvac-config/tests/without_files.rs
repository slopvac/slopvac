//! `LoadedConfig::without_files`: a project that has no `slopvac.toml`.

use std::path::Path;

use serde_json::json;
use slopvac_config::{Inputs, LoadedConfig, NoPresets};

#[test]
fn a_config_without_files_resolves_to_the_core_defaults_and_excludes_nothing() {
    let config = LoadedConfig::without_files(Path::new("/project").to_path_buf());
    let inputs = Inputs {
        presets: &NoPresets,
        front_matter: &[],
        directives: &[],
        cli: &[],
        env: &[],
    };
    let resolved = config.resolve(Path::new("doc.md"), &inputs);
    assert_eq!(resolved.get(&["profile", "name"]), Some(&json!("normal")));
    // The core `exclude` list belongs to a config file: with none, nothing is excluded.
    assert!(!config.is_excluded(Path::new("node_modules/x.md")));
}
