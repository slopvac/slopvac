//! The severity a rule reports at after the config layers.
//!
//! `rules` and `explain` show it next to the severity the rule ships with. The layers come
//! from `--config`, else the nearest `slopvac.toml`, with the bundled style presets
//! underneath (docs/contract/profiles.md, section 3).

use std::path::Path;
use std::sync::Arc;

use clap::ArgMatches;
use slopvac_config::{ConfigError, Discovery, Inputs, Rejected, Resolved, describe, load_labeled};
use slopvac_rules::{LoadedRule, MatchKind};

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

/// Describe a refused config from its structured fields: the first line of each diagnostic,
/// its `file:line:column` pointer and its help. The source excerpt is left out because it
/// quotes the user's own text, paths included. The config crate already words a path in a
/// message as `<external>/<file>` or relative to the project root.
fn describe_rejected(rejected: &Rejected) -> String {
    rejected
        .diagnostics
        .iter()
        .map(|d| {
            let (line, column) = d.location(&rejected.text);
            let mut text = format!("{}\n --> {}:{line}:{column}", d.message, rejected.file);
            if let Some(help) = &d.help {
                text.push_str("\nhelp: ");
                text.push_str(help);
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn config_error(err: &ConfigError) -> CliError {
    let message = match err {
        ConfigError::Rejected(rejected) => describe_rejected(rejected),
        other @ ConfigError::Io { .. } => other.to_string(),
    };
    CliError::new(err.code().as_str(), message)
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
        let qualified_category = format!("{}:{category}", rule.id().pack());
        // A judgement rule has no finding to grade. Only a `rules` entry sets its level; a
        // `categories` entry does not (docs/contract/profiles.md, "Judgement rules").
        let categories: Vec<&str> = if rule.kind() == MatchKind::Judgement {
            Vec::new()
        } else {
            vec![qualified_category.as_str(), category]
        };
        // Either spelling of a key counts, and the layer that wrote the entry decides.
        let named = self
            .resolved
            .level_by_any(&[rule.id().as_str(), rule.id().local()], &categories);
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
