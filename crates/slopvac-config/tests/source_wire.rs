//! `ConfigSource` values built by the config crate match `common.schema.json`.

use jsonschema::Validator;
use serde_json::{Value, json};
use slopvac_config::{ConfigLayer, bare_source, layer_name, located_source};

fn validator() -> Validator {
    let text = include_str!("../../../schemas/v3/common.schema.json");
    let common: Value = serde_json::from_str(text).unwrap();
    jsonschema::draft202012::new(&common["$defs"]["ConfigSource"]).unwrap()
}

const LAYERS: [(ConfigLayer, &str); 10] = [
    (ConfigLayer::Core, "core"),
    (ConfigLayer::Style, "style"),
    (ConfigLayer::Genre, "genre"),
    (ConfigLayer::Extends, "extends"),
    (ConfigLayer::Project, "project"),
    (ConfigLayer::Override, "override"),
    (ConfigLayer::FrontMatter, "front-matter"),
    (ConfigLayer::Directive, "directive"),
    (ConfigLayer::Cli, "cli"),
    (ConfigLayer::Env, "env"),
];

#[test]
fn a_bare_source_serialises_to_the_layer_alone() {
    let validator = validator();
    for (layer, name) in LAYERS {
        let value = serde_json::to_value(bare_source(layer)).unwrap();
        assert_eq!(value, json!({ "layer": name }));
        assert!(validator.is_valid(&value), "{name}");
    }
}

#[test]
fn a_located_source_serialises_file_line_and_column() {
    let source = located_source(ConfigLayer::Project, Some("slopvac.toml".into()), 3, 5);
    let value = serde_json::to_value(source).unwrap();
    assert_eq!(
        value,
        json!({ "layer": "project", "file": "slopvac.toml", "line": 3, "column": 5 })
    );
    assert!(validator().is_valid(&value));
}

#[test]
fn layer_names_match_the_wire_names() {
    for (layer, name) in LAYERS {
        assert_eq!(layer_name(layer), name);
    }
}
