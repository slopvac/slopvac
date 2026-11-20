//! Where a setting came from, and the settings a layer contributes.

use std::num::NonZeroUsize;

use serde_json::Value as Json;
use slopvac_model::{ConfigLayer, ConfigSource};

/// A source with no file, such as `core`, `cli` or `env`.
#[must_use]
pub fn bare_source(layer: ConfigLayer) -> ConfigSource {
    ConfigSource {
        layer,
        file: None,
        line: None,
        column: None,
    }
}

/// A source at a file position. `line` and `column` are one-based; a zero leaves that part out.
#[must_use]
pub fn located_source(
    layer: ConfigLayer,
    file: Option<String>,
    line: usize,
    column: usize,
) -> ConfigSource {
    ConfigSource {
        layer,
        file,
        line: NonZeroUsize::new(line),
        column: NonZeroUsize::new(column),
    }
}

/// The name used in reports and `--explain` output, such as `front-matter`.
#[must_use]
pub fn layer_name(layer: ConfigLayer) -> &'static str {
    match layer {
        ConfigLayer::Core => "core",
        ConfigLayer::Style => "style",
        ConfigLayer::Genre => "genre",
        ConfigLayer::Extends => "extends",
        ConfigLayer::Project => "project",
        ConfigLayer::Override => "override",
        ConfigLayer::FrontMatter => "front-matter",
        ConfigLayer::Directive => "directive",
        ConfigLayer::Cli => "cli",
        ConfigLayer::Env => "env",
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
    pub source: ConfigSource,
}

impl Setting {
    /// A setting with no file, for core defaults, flags and environment switches.
    #[must_use]
    pub fn bare(layer: ConfigLayer, path: &[&str], value: Json) -> Self {
        Self {
            path: path.iter().map(|s| (*s).to_owned()).collect(),
            value,
            source: bare_source(layer),
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
