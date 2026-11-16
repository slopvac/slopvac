//! The engine settings of one file: the profile's legacy style, the project config, and the
//! command line, merged by the precedence in `docs/contract/profiles.md`, section 3.
//!
//! The legacy profile style is the lowest style layer of the same stack as the config files
//! and the listed built-in styles, so the resolver decides every level: a `rules` entry
//! at `off` in the style holds against a category entry above it, and a `minimum_severity`
//! raises an inherited category level. Rule and category keys in the config are first
//! canonicalised against the catalogue, and an unknown name is a config error.
//!
//! STUB(slopvac-8l4.12.29): the genre is the default `reference` unless config sets it, no
//! genre chain or pack style is loaded, and `terms.blocklists` files are not read. The swap
//! bead lists each gap.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use serde_json::Value as Json;
use slopvac_config::{Inputs, Layer, PackFile, Presets};
use slopvac_config::{LoadedConfig, Resolved, Setting, describe};
use slopvac_engine::{CategoryDials, Level, Settings};
use slopvac_model::{
    CategoryId, ConfigLayer, ConfigSource, GenreId, GenreResult, GenreSource, NonEmptyString,
    RuleId, Severity,
};
use slopvac_rules::{MatchKind, RuleSet};
use slopvac_styles::{BuiltinSource, Style, StylePresets};

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
    /// `--disable`: categories and rule ids that do not run.
    pub disable: Vec<String>,
}

const LEGACY: [(&str, &str); 3] = [
    (
        "legacy-strict",
        include_str!("../../../docs/contract/compat/profiles/legacy-strict.toml"),
    ),
    (
        "legacy-normal",
        include_str!("../../../docs/contract/compat/profiles/legacy-normal.toml"),
    ),
    (
        "legacy-relaxed",
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
    styles
        .get(format!("legacy-{profile}").as_str())
        .unwrap_or_else(|| &styles["legacy-normal"])
}

fn rule_id_of(rules: &RuleSet, key: &str) -> Option<RuleId> {
    let qualified = if key.contains(':') {
        key.to_owned()
    } else {
        format!("core:{key}")
    };
    rules
        .rules()
        .find(|r| r.id().as_str() == qualified)
        .map(|r| r.id().clone())
}

fn category_of(rules: &RuleSet, key: &str) -> Option<String> {
    let bare = key.strip_prefix("core:").unwrap_or(key);
    rules
        .categories()
        .iter()
        .find(|c| c.id().as_str() == bare)
        .map(|c| c.id().as_str().to_owned())
}

/// A style layer with its rule and category keys rewritten to the catalogue's names. An
/// entry for a rule or category that this catalogue lacks is dropped: a style is shared data,
/// and the config is where an unknown name is an error.
fn canonical_layer(layer: &[Setting], rules: &RuleSet) -> Vec<Setting> {
    layer
        .iter()
        .filter_map(|setting| {
            let mut setting = setting.clone();
            match setting.path.as_slice() {
                [head, key, ..] if head == "rules" => {
                    let id = rule_id_of(rules, key)?;
                    id.as_str().clone_into(&mut setting.path[1]);
                }
                [head, key, ..] if head == "categories" => {
                    let name = category_of(rules, key)?;
                    setting.path[1] = name;
                }
                _ => {}
            }
            Some(setting)
        })
        .collect()
}

/// Rewrite the rule and category keys of `settings` to the catalogue's names.
fn canonicalise(settings: &mut [Setting], rules: &RuleSet) -> Result<(), CliError> {
    for setting in settings {
        let place = describe(&setting.source);
        match setting.path.as_slice() {
            [head, key, ..] if head == "rules" => {
                let id = rule_id_of(rules, key).ok_or_else(|| {
                    CliError::new(
                        "config-unknown-rule",
                        format!(
                            "{place}: unknown rule {key:?}; use a rule id from `slopvac rules`"
                        ),
                    )
                })?;
                id.as_str().clone_into(&mut setting.path[1]);
            }
            [head, key, ..] if head == "categories" => {
                let name = category_of(rules, key).ok_or_else(|| {
                    CliError::new(
                        "config-unknown-category",
                        format!(
                            "{place}: unknown category {key:?}; use a category from `slopvac rules`"
                        ),
                    )
                })?;
                setting.path[1] = name;
            }
            _ => {}
        }
    }
    Ok(())
}

/// The config with its rule and category keys canonicalised.
///
/// # Errors
/// `config-unknown-rule` or `config-unknown-category`, naming the file and line of the key,
/// and `unknown-style` for a `styles` entry that does not ship.
pub fn canonical_config(config: &LoadedConfig, rules: &RuleSet) -> Result<LoadedConfig, CliError> {
    let listed = config.project_value(&["styles"]);
    for name in listed
        .iter()
        .filter_map(Json::as_array)
        .flatten()
        .filter_map(Json::as_str)
    {
        if !BuiltinSource::names().any(|known| known == name) {
            return Err(CliError::new(
                "unknown-style",
                format!(
                    "slopvac.toml: unknown style {name:?}; the styles that ship are: {}.",
                    BuiltinSource::names().collect::<Vec<_>>().join(", ")
                ),
            ));
        }
    }
    let mut out = config.clone();
    for file in &mut out.files {
        canonicalise(&mut file.settings, rules)?;
        for block in &mut file.blocks {
            canonicalise(&mut block.settings, rules)?;
        }
    }
    Ok(out)
}

/// The presets of one file: the legacy profile style first, then the built-in style
/// presets that `styles` lists. So the profile is the lowest style layer and a listed style
/// outranks it, and the project config outranks both.
struct ProfilePresets<'a> {
    legacy: Vec<Setting>,
    styles: StylePresets<BuiltinSource>,
    rules: &'a RuleSet,
}

impl Presets for ProfilePresets<'_> {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        self.styles
            .style(name)
            .map(|layer| canonical_layer(&layer, self.rules))
    }

    fn style_layers(&self, listed: &[String]) -> Vec<Vec<Setting>> {
        let mut layers = vec![self.legacy.clone()];
        layers.extend(
            self.styles
                .style_layers(listed)
                .iter()
                .map(|layer| canonical_layer(layer, self.rules)),
        );
        layers
    }

    fn genre_chain(&self, name: &str) -> Vec<Vec<Setting>> {
        self.styles.genre_chain(name)
    }

    fn pack_config(&self, name: &str) -> Option<PackFile> {
        self.styles.pack_config(name)
    }
}

/// The merged settings of the file at `rel`, with the legacy style of its profile underneath.
/// `config` is already canonical.
#[must_use]
pub fn resolve(config: &LoadedConfig, rel: &Path, rules: &RuleSet) -> Resolved {
    let styles = StylePresets(BuiltinSource);
    let inputs = |presets: &'_ dyn Presets| {
        config.resolve(
            rel,
            &Inputs {
                presets,
                front_matter: &[],
                directives: &[],
                cli: &[],
                env: &[],
            },
        )
    };
    // The profile can come from an override block, so find it before choosing the style.
    let profile = inputs(&styles)
        .get(&["profile", "name"])
        .and_then(Json::as_str)
        .unwrap_or("normal")
        .to_owned();
    inputs(&ProfilePresets {
        legacy: canonical_layer(legacy_style(&profile).layer(), rules),
        styles,
        rules,
    })
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

fn number(resolved: &Resolved, path: &[&str]) -> Option<f64> {
    resolved.get(path).and_then(Json::as_f64)
}

fn count(resolved: &Resolved, path: &[&str]) -> Option<usize> {
    resolved
        .get(path)
        .and_then(Json::as_u64)
        .and_then(|n| usize::try_from(n).ok())
}

/// Move `level` along suggestion, warning, error by `shift`. `off` stays off.
fn shifted(level: Level, shift: i64) -> Level {
    const RANKS: [Severity; 3] = [Severity::Suggestion, Severity::Warning, Severity::Error];
    let Level::On(severity) = level else {
        return level;
    };
    let Some(at) = RANKS.iter().position(|r| *r == severity) else {
        return level;
    };
    let top = i64::try_from(RANKS.len() - 1).unwrap_or(0);
    let to = (i64::try_from(at).unwrap_or(0) + shift).clamp(0, top);
    Level::On(RANKS[usize::try_from(to).unwrap_or(0)])
}

/// Thresholds: the profile's, then the config's, then the command line's.
fn apply_thresholds(settings: &mut Settings, profile: &str, resolved: &Resolved, cli: &Overrides) {
    let (density, errors, score) = profile_thresholds(profile);
    let t = &mut settings.thresholds;
    t.max_total_per_100_words =
        number(resolved, &["thresholds", "max_total_per_100_words"]).or(density);
    t.max_errors = count(resolved, &["thresholds", "max_errors"]).or(errors);
    t.max_warnings = count(resolved, &["thresholds", "max_warnings"]);
    t.min_score = number(resolved, &["thresholds", "min_score"]).or(score);
    t.max_unicode_dashes =
        Some(count(resolved, &["thresholds", "max_unicode_dashes"]).unwrap_or(0));
    if let Some(n) = cli.min_score {
        t.min_score = Some(n);
    }
    if let Some(n) = cli.max_per_100_words {
        t.max_total_per_100_words = Some(n);
    }
}

fn internal(what: &str, err: impl std::fmt::Display) -> CliError {
    CliError::new("settings-invalid", format!("{what}: {err}"))
}

/// The ids a `--category` or `--disable` name selects: a category, or a rule in either form.
fn selected(rules: &RuleSet, names: &[String]) -> Result<Vec<String>, CliError> {
    let mut out = Vec::new();
    for name in names {
        if let Some(category) = category_of(rules, name) {
            out.push(category);
        } else if let Some(id) = rule_id_of(rules, name) {
            out.push(id.as_str().to_owned());
        } else {
            return Err(CliError::new(
                "usage",
                format!("unknown category or rule {name:?}; see `slopvac rules`"),
            ));
        }
    }
    Ok(out)
}

/// Whether the level came from a rule definition, a style or the genre chain, which the
/// profile's `severity_shift` moves. A level that a config `rules` or `categories` entry
/// sets is final.
fn is_inherited(layer: Layer) -> bool {
    matches!(layer, Layer::Core | Layer::Style | Layer::Genre)
}

/// Build the engine settings for a file from its resolved config.
///
/// `sources` is copied to each report document.
///
/// # Errors
/// `usage` for a `--category` or `--disable` name that the catalogue lacks.
pub fn build(
    rules: &RuleSet,
    resolved: &Resolved,
    sources: Vec<ConfigSource>,
    cli: &Overrides,
) -> Result<Settings, CliError> {
    let profile = resolved
        .get(&["profile", "name"])
        .and_then(Json::as_str)
        .unwrap_or("normal");
    let (genre, source) = match resolved.get(&["genre"]).and_then(Json::as_str) {
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
    apply_thresholds(&mut settings, profile, resolved, cli);

    let shift = resolved
        .get(&["profile", "severity_shift"])
        .and_then(Json::as_i64)
        .unwrap_or(0);
    let disabled = selected(rules, &cli.disable)?;
    let only = selected(rules, &cli.only_categories)?;
    for category in rules.categories() {
        let name = category.id().as_str();
        let budget = resolved
            .get(&["categories", name, "max_per_100_words"])
            .and_then(Json::as_f64);
        settings.categories.insert(
            CategoryId::new(name).map_err(|e| internal("category", e))?,
            CategoryDials {
                level: None,
                weight: number(resolved, &["categories", name, "weight"]),
                max_per_100_words: budget,
            },
        );
        for rule in category.rules() {
            let id = rule.id();
            // A category entry never sets a judgement rule's level: only a `rules` entry does.
            let category_key = (rule.kind() != MatchKind::Judgement).then_some(name);
            let level = match resolved.level(id.as_str(), category_key) {
                Some(named) => level_of(&named.level).map(|l| {
                    if is_inherited(named.source.layer) {
                        shifted(l, shift)
                    } else {
                        l
                    }
                }),
                None if shift != 0 => Some(shifted(Level::On(rule.rule().severity), shift)),
                None => None,
            };
            if let Some(level) = level {
                settings.rule_levels.insert(id.clone(), level);
            }
            let hit = |list: &[String]| list.iter().any(|n| n == name || n.as_str() == id.as_str());
            if hit(&disabled) || (!only.is_empty() && !hit(&only)) {
                settings.rule_levels.insert(id.clone(), Level::Off);
            }
        }
    }

    if let Some(words) = resolved.get(&["terms", "allow"]).and_then(Json::as_array) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use slopvac_config::{NoPresets, load};

    /// The settings that `config` (the text of a `slopvac.toml`) gives to a file.
    fn settings(config: &str) -> Result<Settings, CliError> {
        let rules = crate::rule_catalog::core().expect("the core rules load");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slopvac.toml");
        std::fs::write(&path, config).expect("write config");
        let loaded = load(&path, &NoPresets).expect("the config loads");
        let config = canonical_config(&loaded, &rules)?;
        let resolved = resolve(&config, Path::new("doc.md"), &rules);
        build(&rules, &resolved, Vec::new(), &Overrides::default())
    }

    fn level(settings: &Settings, rule: &str) -> Option<Level> {
        settings
            .rule_levels
            .get(&RuleId::new(rule).expect("a rule id"))
            .copied()
    }

    const HEAD: &str = "schema_version = 3\n";
    const COMMAND_PROMPT: &str = "core:prose-craft.command-prompt";
    const WARNING: Level = Level::On(Severity::Warning);
    const ERROR: Level = Level::On(Severity::Error);

    #[test]
    fn the_profile_turns_a_rule_off_and_a_category_entry_does_not_revive_it() {
        // The `normal` style sets `prose-craft.command-prompt` to off.
        assert_eq!(
            level(&settings(HEAD).unwrap(), COMMAND_PROMPT),
            Some(Level::Off)
        );
        let config = format!("{HEAD}\n[categories]\nprose-craft = \"warning\"\n");
        assert_eq!(
            level(&settings(&config).unwrap(), COMMAND_PROMPT),
            Some(Level::Off)
        );
    }

    #[test]
    fn a_rules_entry_revives_a_rule_the_profile_turns_off() {
        let config = format!("{HEAD}\n[rules]\n\"{COMMAND_PROMPT}\" = \"warning\"\n");
        assert_eq!(
            level(&settings(&config).unwrap(), COMMAND_PROMPT),
            Some(WARNING)
        );
    }

    #[test]
    fn a_minimum_severity_alone_raises_the_inherited_category_level() {
        // `ste-punctuation` is a suggestion category in the `normal` style, and the rule has
        // no entry of its own.
        let rule = "core:ste-punctuation.semicolon-used";
        let base = settings(HEAD).unwrap();
        assert_eq!(level(&base, rule), Some(Level::On(Severity::Suggestion)));
        let config =
            format!("{HEAD}\n[categories.ste-punctuation]\nminimum_severity = \"error\"\n");
        assert_eq!(level(&settings(&config).unwrap(), rule), Some(ERROR));
    }

    #[test]
    fn bare_rule_keys_and_qualified_category_keys_resolve_to_the_catalogue() {
        let config = format!(
            "{HEAD}\n[rules]\n\"prose-inflation.slop-lexicon\" = \"off\"\n\n\
             [categories]\n\"core:orwell\" = \"off\"\n"
        );
        let s = settings(&config).unwrap();
        assert_eq!(
            level(&s, "core:prose-inflation.slop-lexicon"),
            Some(Level::Off)
        );
        assert_eq!(
            level(&s, "core:orwell.unsupported-evaluative"),
            Some(Level::Off)
        );
    }

    #[test]
    fn an_unknown_rule_or_category_is_a_config_error_naming_the_line() {
        let rule = settings(&format!("{HEAD}\n[rules]\n\"orwell.nope\" = \"off\"\n"));
        let error = rule.unwrap_err();
        assert_eq!(error.code, "config-unknown-rule");
        assert!(error.message.contains("slopvac.toml:"), "{}", error.message);
        let category = settings(&format!("{HEAD}\n[categories]\nnope = \"off\"\n"));
        assert_eq!(category.unwrap_err().code, "config-unknown-category");
    }

    #[test]
    fn a_listed_built_in_style_outranks_the_profile_and_an_unknown_style_is_an_error() {
        // The `ste` preset sets this rule to a suggestion, above the profile's category level.
        let rule = "core:ste-nouns.multiword-noun-too-long";
        let with = settings(&format!("{HEAD}styles = [\"ste\"]\n")).unwrap();
        assert_eq!(level(&with, rule), Some(Level::On(Severity::Suggestion)));
        let error = settings(&format!("{HEAD}styles = [\"nope\"]\n")).unwrap_err();
        assert_eq!(error.code, "unknown-style");
    }

    #[test]
    fn severity_shift_moves_inherited_levels_and_keeps_authored_levels_and_off() {
        let config = format!(
            "{HEAD}\n[profile]\nname = \"normal\"\nseverity_shift = 1\n\n[rules]\n\
             \"core:prose-inflation.slop-lexicon\" = \"warning\"\n"
        );
        let s = settings(&config).unwrap();
        // Inherited from the style: warning in `prose-craft`, moved up to error.
        assert_eq!(level(&s, "core:prose-craft.hyphens"), Some(ERROR));
        // Authored in the config: final.
        assert_eq!(
            level(&s, "core:prose-inflation.slop-lexicon"),
            Some(WARNING)
        );
        // Off stays off.
        assert_eq!(level(&s, COMMAND_PROMPT), Some(Level::Off));
        let down = format!("{HEAD}\n[profile]\nname = \"normal\"\nseverity_shift = -1\n");
        let s = settings(&down).unwrap();
        assert_eq!(
            level(&s, "core:prose-craft.hyphens"),
            Some(Level::On(Severity::Suggestion))
        );
        // The lowest level stays where it is.
        assert_eq!(
            level(&s, "core:ste-punctuation.semicolon-used"),
            Some(Level::On(Severity::Suggestion))
        );
    }
}
