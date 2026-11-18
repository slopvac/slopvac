//! The severity a rule reports at after the config layers.
//!
//! `rules` and `explain` show it next to the severity the rule ships with. The layers come
//! from `--config`, else the nearest `slopvac.toml`, with the bundled style presets
//! underneath (docs/contract/profiles.md, section 3).

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use clap::ArgMatches;
use slopvac_config::{ConfigError, Discovery, Inputs, Rejected, Resolved, describe, load_labeled};
use slopvac_rules::{LoadedRule, MatchKind};
use slopvac_styles::{BuiltinSource, StylePresets};

use crate::errors::CliError;
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

/// Redact every backtick-quoted absolute path in `text`.
///
/// The config diagnostics quote the user's own value in backticks, such as the value of a
/// bad `locale` or an unknown key. When the quoted text is an absolute path, it can hold
/// spaces and the directories above the project, so the whole quoted text is replaced with
/// `redact::shown_path` (policies.md, 1.7).
fn redact_quoted(text: &str, cwd: &Path) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let (head, tail) = rest.split_at(open);
        out.push_str(head);
        let tail = &tail[1..];
        let Some(close) = tail.find('`') else {
            out.push('`');
            rest = tail;
            continue;
        };
        let quoted = &tail[..close];
        out.push('`');
        if Path::new(quoted).is_absolute() {
            out.push_str(&redact::shown_path(Path::new(quoted), cwd));
        } else {
            out.push_str(quoted);
        }
        out.push('`');
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Describe a refused config from its structured fields: each diagnostic's message, its
/// `file:line:column` pointer and its help. The source excerpt is left out because it
/// quotes the user's own text, paths included. The first diagnostic's code leads the error;
/// each later one starts with its own `error[code]:` so no code is lost.
fn describe_rejected(rejected: &Rejected, cwd: &Path) -> String {
    rejected
        .diagnostics
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let (line, column) = d.location(&rejected.text);
            let mut text = if i == 0 {
                String::new()
            } else {
                format!("error[{}]: ", d.code.as_str())
            };
            text.push_str(&redact_quoted(&d.message, cwd));
            let _ = write!(text, "\n --> {}:{line}:{column}", rejected.file);
            if let Some(help) = &d.help {
                text.push_str("\nhelp: ");
                text.push_str(&redact_quoted(help, cwd));
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn config_error(err: &ConfigError, cwd: &Path) -> CliError {
    let message = match err {
        ConfigError::Rejected(rejected) => describe_rejected(rejected, cwd),
        other @ (ConfigError::Io { .. } | ConfigError::TooLarge { .. }) => other.to_string(),
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
        let presets = StylePresets(BuiltinSource);
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
                    .map_err(|e| config_error(&e, &cwd))?,
            )
        } else {
            Discovery::new(&presets)
                .nearest(&cwd)
                .map_err(|e| config_error(&e, &cwd))?
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
            if !BuiltinSource::names().any(|n| n == name) {
                return Err(CliError::new(
                    "unknown-style",
                    format!(
                        "unknown style {name:?}; the styles that ship are: {}.",
                        BuiltinSource::names().collect::<Vec<_>>().join(", ")
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
