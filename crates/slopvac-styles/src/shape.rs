//! A style preset file, as `schemas/v3/style.schema.json` defines it.

use std::collections::BTreeMap;

use serde::Deserialize;
use slopvac_config::flatten::{Origin, flatten};
use slopvac_config::tree::parse_toml;
use slopvac_config::{
    Code, Diagnostic, Layer, Setting, SourceFile, render, validate_style, version,
};

use crate::StyleError;
use crate::types::{CategoryPatch, Level, Terms};

/// The file name shown in provenance for the style `name`.
fn origin_name(name: &str) -> String {
    format!("preset:{name}")
}

/// A style preset as written in a data pack.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Style {
    /// One-line description.
    pub description: Option<String>,
    /// Styles resolved before this one, in order.
    #[serde(default)]
    pub extends: Vec<String>,
    /// Styles that this one cannot sit beside without a declared winner.
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// Category dials by category key.
    #[serde(default)]
    pub categories: BTreeMap<String, CategoryPatch>,
    /// Rule levels by rule key.
    #[serde(default)]
    pub rules: BTreeMap<String, Level>,
    /// Terms the style brings.
    #[serde(default)]
    pub terms: Terms,
    /// The style's own settings as a config layer. Each value names `preset:<name>` and the
    /// line of its key.
    #[serde(skip)]
    settings: Vec<Setting>,
}

impl Style {
    /// Parse and validate a style file. Unknown keys that do not start with `x-` are errors.
    ///
    /// # Errors
    /// [`StyleError::Parse`] for bad TOML and [`StyleError::Invalid`] for a file that breaks
    /// the schema, including a `schema_version` other than 3.
    pub fn parse(name: &str, text: &str) -> Result<Self, StyleError> {
        let file = origin_name(name);
        let root = parse_toml(text).map_err(|e| StyleError::Parse {
            style: name.to_owned(),
            message: render(
                SourceFile { name: &file, text },
                &[Diagnostic {
                    code: Code::Parse,
                    message: format!("not valid TOML: {}", e.message),
                    label: "the parser stopped here".into(),
                    span: e.span.unwrap_or(0..0),
                    help: None,
                    path: Vec::new(),
                    data: None,
                }],
            ),
        })?;
        let mut diagnostics = version::check_version(&root, text)
            .err()
            .into_iter()
            .collect::<Vec<_>>();
        if diagnostics.is_empty() {
            diagnostics = validate_style(&root, text);
        }
        if !diagnostics.is_empty() {
            return Err(StyleError::Invalid {
                style: name.to_owned(),
                problems: diagnostics
                    .iter()
                    .map(|d| (d.code.as_str().to_owned(), d.pointer()))
                    .collect(),
                rendered: render(SourceFile { name: &file, text }, &diagnostics),
            });
        }
        let mut style: Self =
            serde_json::from_value(root.to_json()).map_err(|e| StyleError::Parse {
                style: name.to_owned(),
                message: e.to_string(),
            })?;
        style.settings = flatten(
            &root,
            Origin {
                layer: Layer::Style,
                file: &file,
                text,
            },
            &["schema_version", "description", "extends", "conflicts"],
        );
        Ok(style)
    }

    /// The style's own settings as a layer of the config stack, lowest layer first among
    /// the style layers when several apply.
    #[must_use]
    pub fn layer(&self) -> &[Setting] {
        &self.settings
    }
}
