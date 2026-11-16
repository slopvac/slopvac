//! The engine settings of one file: the profile's legacy style, the project config, and the
//! command line, merged by the precedence in `docs/contract/profiles.md`, section 3.
//!
//! STUB(slopvac-8l4.12.21): the genre is the default `reference`, no `styles` or genre chain
//! is loaded (`NoPresets`), and `terms.blocklists` files are not read. The swap bead lists
//! each gap.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value as Json;
use slopvac_config::Resolved;
use slopvac_engine::{CategoryDials, Level, Settings};
use slopvac_model::{
    CategoryId, ConfigLayer, ConfigSource, GenreId, GenreResult, GenreSource, NonEmptyString,
    RuleId, Severity,
};
use slopvac_rules::RuleSet;
use slopvac_styles::{Budget, CategoryPatch, Style};

use crate::errors::CliError;

/// What the command line adds on top of the files.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overrides {
    /// `--min-score`.
    pub min_score: Option<f64>,
    /// `--max-per-100-words`.
    pub max_per_100_words: Option<f64>,
    /// `--category`: when not empty, only these categories run.
    pub only_categories: Vec<String>,
    /// `--disable`: categories and qualified rule ids that do not run.
    pub disable: Vec<String>,
}

const LEGACY: [(&str, &str); 3] = [
    (
        "strict",
        include_str!("../../../docs/contract/compat/profiles/legacy-strict.toml"),
    ),
    (
        "normal",
        include_str!("../../../docs/contract/compat/profiles/legacy-normal.toml"),
    ),
    (
        "relaxed",
        include_str!("../../../docs/contract/compat/profiles/legacy-relaxed.toml"),
    ),
];

/// The thresholds of a profile (`docs/contract/profiles.md`, section 2).
fn profile_thresholds(profile: &str) -> (Option<f64>, Option<usize>, Option<f64>) {
    match profile {
        "strict" => (Some(1.5), Some(0), Some(85.0)),
        "relaxed" => (Some(8.0), None, None),
        _ => (Some(3.0), Some(0), Some(70.0)),
    }
}

fn legacy_style(profile: &str) -> &'static Style {
    static STYLES: OnceLock<BTreeMap<&'static str, Style>> = OnceLock::new();
    let styles = STYLES.get_or_init(|| {
        LEGACY
            .iter()
            .map(|(name, text)| {
                (
                    *name,
                    Style::parse(name, text).expect("the shipped profile style parses"),
                )
            })
            .collect()
    });
    styles.get(profile).unwrap_or_else(|| &styles["normal"])
}

fn level_of(name: &str) -> Option<Level> {
    Some(match name {
        "error" => Level::On(Severity::Error),
        "warning" => Level::On(Severity::Warning),
        "suggestion" => Level::On(Severity::Suggestion),
        "off" => Level::Off,
        _ => return None,
    })
}

fn style_level(level: slopvac_styles::Level) -> Level {
    match level {
        slopvac_styles::Level::Error => Level::On(Severity::Error),
        slopvac_styles::Level::Warning => Level::On(Severity::Warning),
        slopvac_styles::Level::Suggestion => Level::On(Severity::Suggestion),
        slopvac_styles::Level::Off => Level::Off,
    }
}

fn number(resolved: Option<&Resolved>, path: &[&str]) -> Option<f64> {
    resolved?.get(path).and_then(Json::as_f64)
}

fn internal(what: &str, err: impl std::fmt::Display) -> CliError {
    CliError::new("settings-invalid", format!("{what}: {err}"))
}

/// Build the engine settings for a file.
///
/// `resolved` is the merged config of the file, `None` without a config file. `sources` is
/// copied to each report document.
pub fn build(
    rules: &RuleSet,
    resolved: Option<&Resolved>,
    sources: Vec<ConfigSource>,
    cli: &Overrides,
) -> Result<Settings, CliError> {
    let profile = resolved
        .and_then(|r| r.get(&["profile", "name"]))
        .and_then(Json::as_str)
        .unwrap_or("normal");
    let style = legacy_style(profile);

    let (genre, source) = match resolved
        .and_then(|r| r.get(&["genre"]))
        .and_then(Json::as_str)
    {
        Some(name) => (name, GenreSource::Explicit),
        None => ("reference", GenreSource::Default),
    };
    let mut settings = Settings::new(
        NonEmptyString::new(profile).map_err(|e| internal("profile", e))?,
        GenreResult {
            id: GenreId::new(genre).map_err(|e| internal("genre", e))?,
            source,
            reasons: Vec::new(),
        },
    );
    settings.config_sources = sources;

    let (density, errors, score) = profile_thresholds(profile);
    let t = &mut settings.thresholds;
    t.max_total_per_100_words =
        number(resolved, &["thresholds", "max_total_per_100_words"]).or(density);
    t.max_errors = number(resolved, &["thresholds", "max_errors"])
        .map(|n| n as usize)
        .or(errors);
    t.max_warnings = number(resolved, &["thresholds", "max_warnings"]).map(|n| n as usize);
    t.min_score = number(resolved, &["thresholds", "min_score"]).or(score);
    t.max_unicode_dashes =
        Some(number(resolved, &["thresholds", "max_unicode_dashes"]).map_or(0, |n| n as usize));
    if let Some(n) = cli.min_score {
        t.min_score = Some(n);
    }
    if let Some(n) = cli.max_per_100_words {
        t.max_total_per_100_words = Some(n);
    }

    for category in rules.categories() {
        let name = category.id().as_str();
        let base = style
            .settings
            .categories
            .get(name)
            .map(|patch| match patch {
                CategoryPatch::Level(l) => (Some(*l), None, None),
                CategoryPatch::Dials(d) => (d.severity, d.max_per_100_words, d.weight),
            });
        let (base_level, base_budget, base_weight) = base.unwrap_or((None, None, None));
        let budget = match resolved.and_then(|r| {
            r.get(&["categories", name, "max_per_100_words"])
                .map(Json::as_f64)
        }) {
            Some(over) => over,
            None => match base_budget {
                Some(Budget::Limit(n)) => Some(n),
                _ => None,
            },
        };
        let weight = number(resolved, &["categories", name, "weight"]).or(base_weight);
        settings.categories.insert(
            CategoryId::new(name).map_err(|e| internal("category", e))?,
            CategoryDials {
                level: base_level.map(style_level),
                weight,
                max_per_100_words: budget,
            },
        );

        for rule in category.rules() {
            let id: &RuleId = rule.id();
            let local = id.as_str().split_once(':').map_or(id.as_str(), |(_, l)| l);
            let configured = resolved
                .and_then(|r| r.level(id.as_str(), Some(name)))
                .and_then(|l| level_of(&l.level));
            let level = configured
                .or_else(|| style.settings.rules.get(local).map(|l| style_level(*l)))
                .or_else(|| base_level.map(style_level));
            if let Some(level) = level {
                settings.rule_levels.insert(id.clone(), level);
            }
            let named = |list: &[String]| {
                list.iter()
                    .any(|n| n == name || n == id.as_str() || *n == local)
            };
            let off = named(&cli.disable)
                || (!cli.only_categories.is_empty() && !named(&cli.only_categories));
            if off {
                settings.rule_levels.insert(id.clone(), Level::Off);
            }
        }
    }

    if let Some(words) = resolved
        .and_then(|r| r.get(&["terms", "allow"]))
        .and_then(Json::as_array)
    {
        settings.allowed_words = words
            .iter()
            .filter_map(Json::as_str)
            .map(str::to_owned)
            .collect();
    }
    Ok(settings)
}

/// The `config_sources` of a document: the built-in defaults, then each config file.
#[must_use]
pub fn config_sources(files: &[(String, bool)]) -> Vec<ConfigSource> {
    let mut out = vec![ConfigSource {
        layer: ConfigLayer::Core,
        file: None,
        line: None,
        column: None,
    }];
    out.extend(files.iter().map(|(display, extended)| ConfigSource {
        layer: if *extended {
            ConfigLayer::Extends
        } else {
            ConfigLayer::Project
        },
        file: Some(display.clone()),
        line: None,
        column: None,
    }));
    out
}
