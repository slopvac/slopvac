//! Where a setting came from, and the settings a layer contributes.

use serde::Serialize;
use serde_json::Value as Json;

// STUB(slopvac-8l4.4.9): `Layer` and `Source` stand in for the `ConfigSource` model type
// (`common.schema.json#/$defs/ConfigSource`) until `slopvac-model` ships its own.

/// A configuration layer, lowest priority first except `Env`, which holds sticky switches only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layer {
    /// Built-in defaults.
    Core,
    /// A style preset.
    Style,
    /// A genre definition.
    Genre,
    /// An extended config file.
    Extends,
    /// The nearest `slopvac.toml`.
    Project,
    /// A `[[genres]]` or `[[overrides]]` block.
    Override,
    /// The `slopvac:` block of a document.
    FrontMatter,
    /// An inline directive in a document.
    Directive,
    /// A command-line flag.
    Cli,
    /// An environment switch.
    Env,
}

impl Layer {
    /// The name used in reports, such as `front-matter`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Layer::Core => "core",
            Layer::Style => "style",
            Layer::Genre => "genre",
            Layer::Extends => "extends",
            Layer::Project => "project",
            Layer::Override => "override",
            Layer::FrontMatter => "front-matter",
            Layer::Directive => "directive",
            Layer::Cli => "cli",
            Layer::Env => "env",
        }
    }
}

/// The layer and location that set a value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Source {
    /// The layer.
    pub layer: Layer,
    /// Path of the file, relative to the project root. Absent for core, cli and env values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// One-based line of the key that set the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// One-based column of the key that set the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}

impl Source {
    /// A source with no file, such as `core`, `cli` or `env`.
    #[must_use]
    pub fn bare(layer: Layer) -> Self {
        Self {
            layer,
            file: None,
            line: None,
            column: None,
        }
    }
}

/// One leaf value a layer sets: a scalar, or a list that replaces the lower list.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    /// Key path from the document root, such as `["thresholds", "min_score"]`.
    pub path: Vec<String>,
    /// The value.
    pub value: Json,
    /// Where it came from.
    pub source: Source,
}

impl Setting {
    /// A setting with no file, for core defaults, flags and environment switches.
    #[must_use]
    pub fn bare(layer: Layer, path: &[&str], value: Json) -> Self {
        Self {
            path: path.iter().map(|s| (*s).to_owned()).collect(),
            value,
            source: Source::bare(layer),
        }
    }
}

/// The path as dotted text, quoting segments that hold a dot or colon.
#[must_use]
pub fn display_path(path: &[String]) -> String {
    path.iter()
        .map(|seg| {
            if seg.contains(['.', ':', ' ']) {
                format!("\"{seg}\"")
            } else {
                seg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}
