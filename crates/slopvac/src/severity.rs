//! The severity a rule reports at after the config layers.
//!
//! `rules` and `explain` show it next to the severity the rule ships with. The layers come
//! from `--config`, else the nearest `slopvac.toml`, with the bundled style presets
//! underneath (docs/contract/profiles.md, section 3).

use std::path::Path;
use std::sync::Arc;

use clap::ArgMatches;
use slopvac_config::{ConfigError, Discovery, Inputs, LevelFrom, Resolved, describe, load_labeled};
use slopvac_rules::LoadedRule;

use crate::errors::CliError;
use crate::presets::{BuiltinPresets, style_names};
use crate::redact;

/// A rule's severity after the layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    /// `error`, `warning`, `suggestion` or `off`.
    pub level: String,
    /// What the rule ships with.
    pub shipped: String,
    /// The layer and location that changed it, when something did.
    pub from: Option<String>,
}

impl Effective {
    /// Whether the config moved the severity.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.level != self.shipped
    }
}

/// Resolved settings, ready to answer per rule.
pub struct Severities {
    resolved: Resolved,
}

const RANKS: [&str; 3] = ["suggestion", "warning", "error"];

fn config_error(err: &ConfigError) -> CliError {
    let code = err.code().as_str();
    let message = match err {
        ConfigError::Rejected(rejected) => {
            let text = rejected.render();
            // The rendering starts with `error[<code>]: `; the CLI adds that prefix itself.
            text.strip_prefix(&format!("error[{code}]: "))
                .map_or(text.clone(), str::to_owned)
        }
        other => other.to_string(),
    };
    CliError::new(code, message)
}

impl Severities {
    /// Read the config the command names, or the nearest one, and resolve it.
    ///
    /// # Errors
    /// `config-not-found` for a missing `--config` file, `unknown-style` for a style
    /// that does not ship, and the config error code for a file that is refused.
    pub fn load(m: &ArgMatches) -> Result<Self, CliError> {
        let cwd = std::env::current_dir().unwrap_or_default();
        let presets = BuiltinPresets;
        let config = if let Some(path) = m.get_one::<std::path::PathBuf>("config") {
            let shown = redact::shown_path(path, &cwd);
            if !path.is_file() {
                return Err(CliError::new(
                    "config-not-found",
                    format!("--config {shown}: no such file"),
                ));
            }
            Some(
                load_labeled(path, &shown, &presets)
                    .map(Arc::new)
                    .map_err(|e| config_error(&e))?,
            )
        } else {
            Discovery::new(&presets)
                .nearest(&cwd)
                .map_err(|e| config_error(&e))?
        };
        let Some(config) = config else {
            return Ok(Self {
                resolved: Resolved::default(),
            });
        };
        let styles = config.project_value(&["styles"]);
        for name in styles
            .as_ref()
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
        {
            if !BuiltinPresets::has(name) {
                return Err(CliError::new(
                    "unknown-style",
                    format!(
                        "unknown style {name:?}; the styles that ship are: {}.",
                        style_names().join(", ")
                    ),
                ));
            }
        }
        let inputs = Inputs {
            presets: &presets,
            front_matter: &[],
            directives: &[],
            cli: &[],
            env: &[],
        };
        Ok(Self {
            resolved: config.resolve(Path::new(""), &inputs),
        })
    }

    /// The effective severity of `rule`.
    ///
    /// A `rules` or `categories` entry sets the level outright. Otherwise the profile's
    /// `severity_shift` moves the shipped level along suggestion, warning, error.
    #[must_use]
    pub fn of(&self, rule: &LoadedRule, shipped: &str) -> Effective {
        let category = rule.category().as_str();
        // A rule may be keyed `core:cat.id` or `cat.id`. A rules entry under either key
        // beats a category entry, so look at both before settling for the category.
        let by_qualified = self.resolved.level(rule.id().as_str(), Some(category));
        let by_bare = self.resolved.level(rule.id().local(), Some(category));
        let named = [by_qualified, by_bare]
            .into_iter()
            .flatten()
            .reduce(|first, second| {
                if first.from == LevelFrom::Rule || second.from != LevelFrom::Rule {
                    first
                } else {
                    second
                }
            });
        if let Some(named) = named {
            return Effective {
                level: named.level,
                shipped: shipped.to_owned(),
                from: Some(describe(&named.source)),
            };
        }
        let shift = self
            .resolved
            .get(&["profile", "severity_shift"])
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let moved = RANKS
            .iter()
            .position(|r| *r == shipped)
            .filter(|_| shift != 0)
            .map(|at| {
                let top = i64::try_from(RANKS.len() - 1).unwrap_or(0);
                let to = (i64::try_from(at).unwrap_or(0) + shift).clamp(0, top);
                RANKS[usize::try_from(to).unwrap_or(0)]
            });
        match moved {
            Some(level) => Effective {
                level: level.to_owned(),
                shipped: shipped.to_owned(),
                from: self
                    .resolved
                    .source(&["profile", "severity_shift"])
                    .map(|s| format!("profile severity_shift from {}", describe(s))),
            },
            None => Effective {
                level: shipped.to_owned(),
                shipped: shipped.to_owned(),
                from: None,
            },
        }
    }
}
