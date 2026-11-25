//! The severity a rule reports at after the config layers.
//!
//! `rules` and `explain` show it next to the severity the rule ships with. The layers come
//! from `--config`, else the nearest `slopvac.toml`, with the bundled style presets
//! underneath (docs/contract/profiles.md, section 3).

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use clap::ArgMatches;
use slopvac_config::{
    ConfigError, Discovery, Inputs, LoadedConfig, Rejected, Resolved, describe, load_labeled,
};
use slopvac_rules::{LoadedRule, MatchKind};
use slopvac_styles::BuiltinSource;

use crate::errors::CliError;
use crate::lint_settings::ShippedStyles;
use crate::redact;
use crate::rule_catalog::Catalog;

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
    /// Bare names that more than one plugin ships: category ids and local rule ids. A bare
    /// config key for such a name names none of them.
    shared: HashSet<String>,
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
        let presets = ShippedStyles::new();
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
            return Ok(Self::unconfigured());
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
        Ok(Self::from_config(&config))
    }

    /// Resolve `config` with the shipped styles and genres.
    #[must_use]
    pub fn from_config(config: &LoadedConfig) -> Self {
        let inputs = Inputs {
            presets: &ShippedStyles::new(),
            front_matter: &[],
            directives: &[],
            cli: &[],
            env: &[],
        };
        Self {
            resolved: config.resolve(Path::new(""), &inputs),
            shared: HashSet::new(),
        }
    }

    /// `name` when a bare config key may name it: no other plugin shares it.
    fn bare<'n>(&self, name: &'n str) -> Option<&'n str> {
        (!self.shared.contains(name)).then_some(name)
    }

    /// Severities for a run with no config.
    #[must_use]
    pub fn unconfigured() -> Self {
        Self {
            resolved: Resolved::default(),
            shared: HashSet::new(),
        }
    }

    /// Note which bare names more than one plugin of `rules` ships, so that a bare config
    /// key does not reach the rules of every plugin that shares the name.
    #[must_use]
    pub fn for_catalog(mut self, rules: &Catalog) -> Self {
        self.shared = rules
            .categories()
            .map(|c| c.id().as_str())
            .filter(|name| rules.categories_named(name).len() > 1)
            .chain(
                rules
                    .rules()
                    .map(|r| r.id().local())
                    .filter(|local| rules.rules_named(local).len() > 1),
            )
            .map(str::to_owned)
            .collect();
        self
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
        let bare = |name| self.bare(name);
        let mut categories: Vec<&str> = Vec::new();
        if rule.kind() != MatchKind::Judgement {
            categories.push(qualified_category.as_str());
            categories.extend(bare(category));
        }
        let mut keys = vec![rule.id().as_str()];
        keys.extend(bare(rule.id().local()));
        // Either spelling of a key counts, and the layer that wrote the entry decides.
        let named = self.resolved.level_by_any(&keys, &categories);
        if let Some(named) = named {
            return Effective {
                level: named.level.as_str().to_owned(),
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
    use crate::rule_catalog::catalog_of;
    use rstest::rstest;

    /// The severities that the config text `body` gives, over `rules`.
    fn severities(rules: &Catalog, body: &str) -> Severities {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slopvac.toml");
        std::fs::write(&path, format!("schema_version = 3\n{body}")).expect("write");
        let config = slopvac_config::load(&path, &ShippedStyles::new()).expect("the config loads");
        Severities::from_config(&config).for_catalog(rules)
    }

    /// The levels of `alpha:style.hit` and `beta:style.hit` that `categories` gives them, over
    /// two plugins that each ship a category `style` whose rule ships as a warning.
    fn levels(categories: &str) -> (String, String) {
        let rules = catalog_of(&[("alpha", "style"), ("beta", "style")]);
        let severities = severities(&rules, categories);
        let level = |id: &str| {
            let id = slopvac_model::RuleId::new(id).expect("a rule id");
            let rule = rules.get(&id).expect("the rule exists");
            severities.of(rule, "warning").level
        };
        (level("alpha:style.hit"), level("beta:style.hit"))
    }

    #[rstest]
    #[case::bare_key_shared_by_two_plugins(
        "[categories.style]\nseverity = \"error\"\n",
        "warning",
        "warning"
    )]
    #[case::qualified_key_names_one_plugin(
        "[categories.\"alpha:style\"]\nseverity = \"error\"\n",
        "error",
        "warning"
    )]
    fn a_category_key_reaches_only_the_plugin_it_names(
        #[case] config: &str,
        #[case] alpha: &str,
        #[case] beta: &str,
    ) {
        assert_eq!(levels(config), (alpha.to_owned(), beta.to_owned()));
    }

    /// The shipped `ste` style turns `ste-practices.latin-abbreviation` off by its bare name,
    /// and a plugin ships a rule with the same local id. The shipped rule is `core`'s.
    #[test]
    fn a_shipped_preset_rule_level_stays_with_core_when_a_plugin_shares_the_rule_id() {
        let rules = crate::lint_settings::tests::core_and_a_plugin_sharing_a_preset_rule();
        let severities = severities(&rules, "styles = [\"ste\"]\n");
        let level = |id: &str| {
            let id = slopvac_model::RuleId::new(id).expect("a rule id");
            let rule = rules.get(&id).expect("the rule exists");
            severities.of(rule, "warning").level
        };
        assert_eq!(level("core:ste-practices.latin-abbreviation"), "off");
        assert_eq!(level("alpha:ste-practices.latin-abbreviation"), "warning");
    }
}
