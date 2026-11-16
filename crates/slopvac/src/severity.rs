//! The severity a rule reports at after the config layers.
//!
//! `rules` and `explain` show it next to the severity the rule ships with. The layers come
//! from `--config`, else the nearest `slopvac.toml`, with the bundled style presets
//! underneath (docs/contract/profiles.md, section 3).

use std::path::Path;
use std::sync::Arc;

use clap::ArgMatches;
use slopvac_config::{ConfigError, Discovery, Inputs, Resolved, describe, load_labeled};
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

/// Drop the source excerpt of a rendered diagnostic and replace absolute paths in the rest.
///
/// What stays is the code line, the `file:line:column` pointer and the help text. A path the
/// user wrote in the file, such as an `extends` target, appears in them only in redacted form
/// (policies.md, 1.7).
fn sanitize(text: &str, cwd: &Path) -> String {
    text.lines()
        .filter(|line| !is_excerpt(line))
        .map(|line| redact_paths(line, cwd))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A gutter line of the rendering: `  |`, `2 | extends = ...`.
fn is_excerpt(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with('|') {
        return true;
    }
    t.split_once(' ').is_some_and(|(number, rest)| {
        !number.is_empty()
            && number.bytes().all(|b| b.is_ascii_digit())
            && rest.trim_start().starts_with('|')
    })
}

/// Replace every absolute path in `line` with `redact::shown_path`.
fn redact_paths(line: &str, cwd: &Path) -> String {
    let is_end = |c: char| c.is_whitespace() || matches!(c, '`' | '\'' | '"' | ')' | ']');
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find('/') {
        let starts_token = at == 0
            || rest[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || matches!(c, '`' | '\'' | '"' | '(' | '['));
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        if !starts_token {
            out.push('/');
            rest = &tail[1..];
            continue;
        }
        let end = tail.find(is_end).unwrap_or(tail.len());
        let token = &tail[..end];
        let trimmed = token.trim_end_matches(['.', ',', ':', ';']);
        out.push_str(&redact::shown_path(Path::new(trimmed), cwd));
        out.push_str(&token[trimmed.len()..]);
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn config_error(err: &ConfigError, cwd: &Path) -> CliError {
    let code = err.code().as_str();
    let message = match err {
        ConfigError::Rejected(rejected) => {
            let text = rejected.render();
            // The rendering starts with `error[<code>]: `; the CLI adds that prefix itself.
            let body = text
                .strip_prefix(&format!("error[{code}]: "))
                .unwrap_or(&text);
            sanitize(body, cwd)
        }
        other @ ConfigError::Io { .. } => sanitize(&other.to_string(), cwd),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_lines_are_dropped_and_the_pointer_stays() {
        let text = "cannot read `/srv/x/base.toml`: gone\n --> slopvac.toml:2:12\n  |\n2 | extends = [\"/srv/x/base.toml\"]\n  |            ^^^ extended here\n  |\nhelp: paths are relative";
        assert_eq!(
            sanitize(text, Path::new("/work")),
            "cannot read `<external>/base.toml`: gone\n --> slopvac.toml:2:12\nhelp: paths are relative"
        );
    }

    #[test]
    fn a_path_inside_the_working_directory_becomes_relative() {
        assert_eq!(
            redact_paths("see /work/conf/a.toml.", Path::new("/work")),
            "see conf/a.toml."
        );
    }

    #[test]
    fn a_slash_inside_a_word_is_not_a_path() {
        assert_eq!(
            redact_paths("read/write and a/b", Path::new("/work")),
            "read/write and a/b"
        );
    }
}
