//! Styles as layer 2 of the config stack: values keep the preset and line that set them.

use std::path::Path;

use serde_json::json;
use slopvac_config::resolve::describe;
use slopvac_config::{Inputs, Layer, LoadedConfig, load, render_explain};
use slopvac_styles::{BuiltinSource, MapSource, StylePresets, StyleSource};

fn resolve_project(config: &str) -> (LoadedConfig, slopvac_config::Resolved) {
    resolve_with(BuiltinSource, config)
}

fn resolve_with(
    source: impl StyleSource,
    config: &str,
) -> (LoadedConfig, slopvac_config::Resolved) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("slopvac.toml");
    std::fs::write(&path, config).unwrap();
    let presets = StylePresets(source);
    let loaded = load(&path, &presets).unwrap();
    let inputs = Inputs {
        presets: &presets,
        front_matter: &[],
        directives: &[],
        cli: &[],
        env: &[],
    };
    let resolved = loaded.resolve(Path::new("doc.md"), &inputs);
    (loaded, resolved)
}

#[test]
fn a_value_from_a_preset_names_the_preset_and_its_line() {
    let (_, resolved) = resolve_project("schema_version = 3\nstyles = [\"ste\"]\n");
    // presets/ste.toml line 32: "ste-verbs.passive-voice" = "suggestion"
    let source = resolved
        .source(&["rules", "ste-verbs.passive-voice"])
        .unwrap();
    assert_eq!(source.layer, Layer::Style);
    assert_eq!(source.file.as_deref(), Some("preset:ste"));
    assert_eq!((source.line, source.column), (Some(32), Some(1)));
    assert_eq!(describe(source), "style preset:ste:32:1");
}

#[test]
fn a_dial_inside_an_inline_table_points_at_its_own_key() {
    let (_, resolved) = resolve_project("schema_version = 3\nstyles = [\"ste\"]\n");
    // presets/ste.toml line 11: the `weight` key of ste-safety starts at column 61
    let source = resolved
        .source(&["categories", "ste-safety", "weight"])
        .unwrap();
    assert_eq!((source.line, source.column), (Some(11), Some(61)));
    assert_eq!(
        resolved.get(&["categories", "ste-safety", "weight"]),
        Some(&json!(1.2))
    );
}

#[test]
fn explain_shows_a_project_value_replacing_a_preset_value() {
    let (loaded, resolved) = resolve_project(
        "schema_version = 3\nstyles = [\"ste\"]\n[rules]\n\"ste-verbs.passive-voice\" = \"error\"\n",
    );
    let explained = resolved.explain();
    let text = render_explain(&loaded, &explained);
    let line = text
        .lines()
        .position(|l| l.starts_with("rules.\"ste-verbs.passive-voice\" = "))
        .expect("the key is explained");
    let block: Vec<&str> = text.lines().skip(line).take(2).collect();
    assert_eq!(
        block,
        [
            "rules.\"ste-verbs.passive-voice\" = \"error\"  [project slopvac.toml:4:1]",
            "    replaced \"suggestion\"  [style preset:ste:32:1]",
        ]
    );
}

#[test]
fn the_later_listed_style_wins_and_the_earlier_one_stays_in_the_history() {
    let (_, resolved) = resolve_project("schema_version = 3\nstyles = [\"google\", \"ste\"]\n");
    // google, then ste: ste wins `latin-abbreviation`, google's value is the replaced one.
    let key = ["rules", "ste-practices.latin-abbreviation"];
    let files: Vec<_> = resolved
        .history(&key)
        .iter()
        .map(|c| c.source.file.clone().unwrap())
        .collect();
    assert_eq!(files, ["preset:google", "preset:ste"]);
    assert_eq!(resolved.get(&key), Some(&json!("off")));
}

const PARENT: &str = "schema_version = 3\n[rules]\n\"core:a\" = \"warning\"\n";

#[test]
fn a_shared_parent_does_not_restore_a_value_the_earlier_style_overrode() {
    let source = MapSource::new()
        .with("parent", PARENT)
        .with(
            "left",
            "schema_version = 3\nextends = [\"parent\"]\n[rules]\n\"core:a\" = \"off\"\n",
        )
        .with("right", "schema_version = 3\nextends = [\"parent\"]\n");
    let (_, resolved) = resolve_with(
        source,
        "schema_version = 3\nstyles = [\"left\", \"right\"]\n",
    );
    // Standalone order is parent, left, right: left's `off` is the last word.
    assert_eq!(resolved.get(&["rules", "core:a"]), Some(&json!("off")));
    let files: Vec<_> = resolved
        .history(&["rules", "core:a"])
        .iter()
        .map(|c| c.source.file.clone().unwrap())
        .collect();
    assert_eq!(files, ["preset:parent", "preset:left"]);
}

#[test]
fn a_child_category_level_beats_its_parents_rule_level() {
    let source = MapSource::new().with("parent", PARENT).with(
        "child",
        "schema_version = 3\nextends = [\"parent\"]\n[categories]\nc = \"error\"\n",
    );
    let (_, resolved) = resolve_with(source, "schema_version = 3\nstyles = [\"child\"]\n");
    // The child is a higher layer than its parent, so its category entry outranks the
    // parent's rule entry.
    let level = resolved.level("core:a", Some("c")).unwrap();
    assert_eq!(level.level, "error");
    assert_eq!(level.source.file.as_deref(), Some("preset:child"));
}

#[test]
fn a_style_file_one_byte_over_the_cap_is_refused_with_its_code() {
    use slopvac_styles::{DirSource, MAX_STYLE_BYTES, StyleError, StyleSource};

    let dir = tempfile::tempdir().unwrap();
    std::fs::File::create(dir.path().join("big.toml"))
        .unwrap()
        .set_len(MAX_STYLE_BYTES + 1)
        .unwrap();
    let err = DirSource::new(dir.path()).load("big").unwrap_err();
    assert_eq!(
        err,
        StyleError::TooLarge {
            style: "big".into(),
            cap: MAX_STYLE_BYTES
        }
    );
    assert!(err.to_string().starts_with("[style-too-large]"), "{err}");
}

#[test]
fn an_oversized_style_file_fails_the_config_load_with_its_code() {
    use slopvac_styles::{DirSource, MAX_STYLE_BYTES};

    let styles = tempfile::tempdir().unwrap();
    std::fs::File::create(styles.path().join("big.toml"))
        .unwrap()
        .set_len(MAX_STYLE_BYTES + 1)
        .unwrap();
    let project = tempfile::tempdir().unwrap();
    let path = project.path().join("slopvac.toml");
    std::fs::write(&path, "schema_version = 3\nstyles = [\"big\"]\n").unwrap();
    let err = load(&path, &StylePresets(DirSource::new(styles.path()))).unwrap_err();
    assert_eq!(err.code(), slopvac_config::Code::StyleTooLarge);
    assert!(err.render().contains("style-too-large"), "{}", err.render());
}
