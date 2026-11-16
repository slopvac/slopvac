//! `LoadedConfig::without_files` and `with_base_style`: a style layer under the config files.

use std::path::Path;

use serde_json::json;
use slopvac_config::{Inputs, Layer, LoadedConfig, NoPresets, Setting, load};

fn inputs() -> Inputs<'static> {
    Inputs {
        presets: &NoPresets,
        front_matter: &[],
        directives: &[],
        cli: &[],
        env: &[],
    }
}

fn style(rule: &str, level: &str) -> Vec<Setting> {
    vec![Setting::bare(Layer::Style, &["rules", rule], json!(level))]
}

#[test]
fn a_config_without_files_resolves_the_base_style_over_the_core_defaults() {
    let config = LoadedConfig::without_files(Path::new("/project").to_path_buf())
        .with_base_style("profile normal", style("core:a.b", "warning"));
    let resolved = config.resolve(Path::new("doc.md"), &inputs());
    assert_eq!(
        resolved.get(&["rules", "core:a.b"]),
        Some(&json!("warning"))
    );
    assert_eq!(resolved.get(&["profile", "name"]), Some(&json!("normal")));
    assert!(!config.is_excluded(Path::new("node_modules/x.md")));
}

#[test]
fn the_project_config_outranks_the_base_style() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("slopvac.toml");
    std::fs::write(
        &path,
        "schema_version = 3\n\n[rules]\n\"core:a.b\" = \"error\"\n",
    )
    .expect("write");
    let loaded = load(&path, &NoPresets).expect("loads");
    let config = loaded.with_base_style("profile normal", style("core:a.b", "off"));
    let resolved = config.resolve(Path::new("doc.md"), &inputs());
    let level = resolved.level("core:a.b", None).expect("a level");
    assert_eq!(level.level, "error");
    assert_eq!(level.source.layer, Layer::Project);
    // The base style is still the lowest layer in the history.
    let history = resolved.history(&["rules", "core:a.b"]);
    assert_eq!(history.first().map(|c| c.source.layer), Some(Layer::Style));
}
