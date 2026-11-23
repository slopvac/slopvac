//! The engine settings of one file: the project config and the command line, merged by the
//! precedence in `docs/contract/profiles.md`, section 3.
//!
//! The profile name sets the thresholds and the `severity_shift`. The 2.x category and rule
//! levels of a profile are the styles `core:legacy-<profile>`, which apply only when a config
//! lists them. Without one, rules report at the levels of their definitions. Config and
//! styles share one layer stack, so the resolver decides every level. An unknown category name
//! in the config is an error, and an unknown rule id is a warning notice.
//!
//! STUB(slopvac-8l4.12.29): the genre is the default `reference` unless config sets it, no
//! genre chain or pack style is loaded, and `terms.blocklists` files are not read. The swap
//! bead lists each gap.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use serde_json::Value as Json;
use slopvac_config::{ConfigLevel, LevelFrom, LoadedConfig, Resolved, Setting, describe};
use slopvac_config::{Inputs, PackFile, Presets, UnknownRule, unknown_rules};
use slopvac_context::GenreAssignment;
use slopvac_engine::{CategoryDials, Level, Settings};
use slopvac_model::{
    CategoryId, ConfigLayer, ConfigSource, GenreId, GenreResult, GenreSource, KebabName,
    NonEmptyString, Notice, NoticeLevel, RuleId, Severity,
};
use slopvac_rules::MatchKind;
use slopvac_styles::{BuiltinSource, Style, StylePresets};

use crate::errors::CliError;
use crate::rule_catalog::Catalog;

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

fn rule_id_of(rules: &Catalog, key: &str) -> Option<RuleId> {
    rules.rule_named(key).map(|r| r.id().clone())
}

fn category_of(rules: &Catalog, key: &str) -> Option<String> {
    rules
        .category_named(key)
        .map(|c| c.id().as_str().to_owned())
}

/// Check the names a config uses against the catalogue. A key may be bare (`orwell.x`,
/// `orwell`) or qualified (`core:orwell.x`, `core:orwell`); both resolve to the same rule or
/// category, and the layer that wrote an entry decides which wins, never its spelling.
///
/// An unknown rule id is collected in `unknown`, since the run goes on without it. An unknown
/// category stops the run.
fn check_names(
    settings: &[Setting],
    rules: &Catalog,
    root: &Path,
    unknown: &mut Vec<UnknownRule>,
) -> Result<(), Box<Located>> {
    for setting in settings {
        if let [head, key, ..] = setting.path.as_slice()
            && head == "categories"
            && category_of(rules, key).is_none()
        {
            return Err(Box::new(Located {
                error: CliError::escaping(
                    "config-unknown-category",
                    &format!(
                        "{}: unknown category {key:?}; use a category from `slopvac rules`",
                        describe(&redacted(&setting.source, root))
                    ),
                ),
                source: Some(setting.source.clone()),
            }));
        }
    }
    unknown.extend(unknown_rules(settings, |key| {
        rule_id_of(rules, key).is_some()
    }));
    Ok(())
}

/// The warning notice of a rule id that no loaded rule has. It names the config file as the
/// report may show it (policies.md, 1.7) and carries `rule`, `line` and `column` in its data.
fn unknown_rule_notice(unknown: &UnknownRule) -> Result<Notice, CliError> {
    let source = unknown.source();
    let mut data = serde_json::Map::new();
    data.insert("rule".into(), unknown.rule().into());
    if let Some(line) = source.line {
        data.insert("line".into(), line.get().into());
    }
    if let Some(column) = source.column {
        data.insert("column".into(), column.get().into());
    }
    Ok(Notice {
        code: KebabName::new(unknown.code()).map_err(|e| internal("notice code", e))?,
        level: NoticeLevel::Warning,
        message: NonEmptyString::new(unknown.message()).map_err(|e| internal("notice", e))?,
        path: unknown.shown_file(),
        data,
    })
}

/// A config failure and the place in the config it is about, when it has one. The error
/// shows the place redacted, and `source` keeps its identity for a caller that must find the
/// file again.
#[derive(Debug)]
pub struct Located {
    /// The failure, as `slopvac lint` reports it.
    pub error: CliError,
    /// Where in the config the failure is.
    pub source: Option<ConfigSource>,
}

/// `source` with its file shown as policies section 1.7 says: relative to `root` under it,
/// else `<external>/<file name>`.
fn redacted(source: &ConfigSource, root: &Path) -> ConfigSource {
    let mut shown = source.clone();
    shown.file = source
        .file
        .as_ref()
        .map(|file| slopvac_fs::display_under_root(&root.join(file), root));
    shown
}

/// Check a config against the catalogue.
///
/// Returns one `config-unknown-rule` warning notice for each rule id that a config file, an
/// `extends` target or a block names and no rule has.
///
/// # Errors
/// `config-unknown-category`, naming the file and line of the key, and `unknown-style` for a
/// `styles` entry that does not ship.
pub fn validate(config: &LoadedConfig, rules: &Catalog) -> Result<Vec<Notice>, CliError> {
    validate_located(config, rules).map_err(|located| located.error)
}

/// As [`validate`], with the place of a failure.
///
/// # Errors
/// As [`validate`]; `source` names the key of a `config-unknown-category`.
pub fn validate_located(
    config: &LoadedConfig,
    rules: &Catalog,
) -> Result<Vec<Notice>, Box<Located>> {
    let listed = config.project_value(&["styles"]);
    for name in listed
        .iter()
        .filter_map(Json::as_array)
        .flatten()
        .filter_map(Json::as_str)
    {
        if legacy_name(name).is_none() && !BuiltinSource::names().any(|known| known == name) {
            return Err(Box::new(Located {
                error: CliError::new(
                    "unknown-style",
                    format!(
                        "slopvac.toml: unknown style {name:?}; the styles that ship are: {}, \
                         core:legacy-strict, core:legacy-normal, core:legacy-relaxed.",
                        BuiltinSource::names().collect::<Vec<_>>().join(", ")
                    ),
                ),
                // The `styles` setting that won is the one with the bad name.
                source: config
                    .files
                    .iter()
                    .flat_map(|file| file.settings.iter())
                    .rfind(|setting| setting.path == ["styles"])
                    .map(|setting| setting.source.clone()),
            }));
        }
    }
    let mut unknown = Vec::new();
    for file in &config.files {
        check_names(&file.settings, rules, &config.root, &mut unknown)?;
        for block in &file.blocks {
            check_names(&block.settings, rules, &config.root, &mut unknown)?;
        }
    }
    // A config that `extends` reaches twice, by a repeated entry or a diamond, is applied
    // twice. Its keys are the same keys, so each id at each source location reports once.
    let unknown = without_repeats(unknown);
    unknown
        .iter()
        .map(unknown_rule_notice)
        .collect::<Result<_, _>>()
        .map_err(|error| {
            Box::new(Located {
                error,
                source: None,
            })
        })
}

/// Drop the repeats of a rule id at one source: the same layer, file, line and column. The
/// source is compared before it is shown, so two files with the same name stay apart, and the
/// same position in two layers stays two findings.
fn without_repeats(mut unknown: Vec<UnknownRule>) -> Vec<UnknownRule> {
    let mut seen = HashSet::new();
    unknown.retain(|u| {
        let source = u.source();
        seen.insert((
            u.rule().to_owned(),
            source.layer,
            source.file.clone(),
            source.line,
            source.column,
        ))
    });
    unknown
}

/// The name a config lists to ask for the 2.x levels of a profile: `core:legacy-normal`,
/// `core:legacy-strict` or `core:legacy-relaxed`. `slopvac migrate --preserve-behavior`
/// writes it. Nothing applies it by default (docs/contract/profiles.md, section 1).
fn legacy_name(listed: &str) -> Option<&'static Style> {
    let profile = listed.strip_prefix("core:legacy-")?;
    LEGACY
        .iter()
        .any(|(name, _)| name.strip_prefix("legacy-") == Some(profile))
        .then(|| legacy_style(profile))
}

/// The built-in style presets, plus the `core:legacy-<profile>` styles.
struct ShippedStyles {
    styles: StylePresets<BuiltinSource>,
}

impl Presets for ShippedStyles {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        match legacy_name(name) {
            Some(style) => Some(style.layer().to_vec()),
            None => self.styles.style(name),
        }
    }

    fn style_layers(&self, listed: &[String]) -> Vec<Vec<Setting>> {
        // Runs of ordinary styles go to the style stack together, so a shared parent
        // applies once. A legacy style is one layer at its place in the list.
        let mut layers = Vec::new();
        let mut run: Vec<String> = Vec::new();
        for name in listed {
            if let Some(style) = legacy_name(name) {
                if !run.is_empty() {
                    layers.extend(self.styles.style_layers(&std::mem::take(&mut run)));
                }
                layers.push(style.layer().to_vec());
            } else {
                run.push(name.clone());
            }
        }
        if !run.is_empty() {
            layers.extend(self.styles.style_layers(&run));
        }
        layers
    }

    fn genre_chain(&self, name: &str) -> Vec<Vec<Setting>> {
        self.styles.genre_chain(name)
    }

    fn pack_config(&self, name: &str) -> Option<PackFile> {
        self.styles.pack_config(name)
    }
}

/// The `[[genres]]` assignments of `config`, in the order they apply, for the engine to match
/// against each document's root-relative path.
#[must_use]
pub fn genre_assignments(config: &LoadedConfig) -> Vec<GenreAssignment> {
    config
        .genre_assignments()
        .into_iter()
        .map(|(files, genre)| GenreAssignment {
            files: files.to_vec(),
            genre: genre.to_owned(),
        })
        .collect()
}

/// The merged settings of the file at `rel`.
#[must_use]
pub fn resolve(config: &LoadedConfig, rel: &Path) -> Resolved {
    config.resolve(
        rel,
        &Inputs {
            presets: &ShippedStyles {
                styles: StylePresets(BuiltinSource),
            },
            front_matter: &[],
            directives: &[],
            cli: &[],
            env: &[],
        },
    )
}

/// The engine level for a config level. A config level this build does not know sets none, so
/// the rule keeps the level of its definition.
fn engine_level(level: ConfigLevel) -> Option<Level> {
    match level {
        ConfigLevel::Off => Some(Level::Off),
        ConfigLevel::On(severity) => Some(Level::On(severity)),
        _ => None,
    }
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

const RANKS: [&str; 4] = ["off", "suggestion", "warning", "error"];

fn rank(name: &str) -> Option<usize> {
    RANKS.iter().position(|r| *r == name)
}

/// Raise `level` to `floor`. `off` as the floor sets none.
fn floored(level: Level, floor: Option<&str>) -> Level {
    let (Level::On(severity), Some(floor)) = (level, floor) else {
        return level;
    };
    let have = rank(match severity {
        Severity::Suggestion => "suggestion",
        Severity::Warning => "warning",
        Severity::Error => "error",
    });
    match (have, rank(floor), level_of(floor)) {
        (Some(h), Some(f), Some(raised)) if f > 1 && h < f => raised,
        _ => level,
    }
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
fn selected(rules: &Catalog, names: &[String]) -> Result<Vec<String>, CliError> {
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
fn is_inherited(layer: ConfigLayer) -> bool {
    matches!(
        layer,
        ConfigLayer::Core | ConfigLayer::Style | ConfigLayer::Genre
    )
}

/// Build the engine settings for a file from its resolved config.
///
/// `sources` is copied to each report document.
///
/// # Errors
/// `usage` for a `--category` or `--disable` name that the catalogue lacks.
pub fn build(
    rules: &Catalog,
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
    settings.severity_shift = resolved
        .get(&["profile", "severity_shift"])
        .and_then(Json::as_i64)
        .unwrap_or(0);

    let shift = resolved
        .get(&["profile", "severity_shift"])
        .and_then(Json::as_i64)
        .unwrap_or(0);
    let disabled = selected(rules, &cli.disable)?;
    let only = selected(rules, &cli.only_categories)?;
    for category in rules.categories() {
        let name = category.id().as_str();
        let qualified = format!(
            "{}:{name}",
            category.rules().first().map_or("core", |r| r.id().pack())
        );
        let field = |f: &'static str| {
            vec![
                vec!["categories", qualified.as_str(), f],
                vec!["categories", name, f],
            ]
        };
        // `"none"` clears the budget; it is stored as null and must not fall back to the
        // budget in the rule data.
        let budget = resolved.get_by_any(&field("max_per_100_words"));
        settings.categories.insert(
            CategoryId::new(name).map_err(|e| internal("category", e))?,
            CategoryDials::default()
                .with_weight(resolved.get_by_any(&field("weight")).and_then(Json::as_f64))
                .with_max_per_100_words(budget.and_then(Json::as_f64))
                .with_budget_cleared(budget.is_some_and(Json::is_null)),
        );
        let floor = resolved
            .get_by_any(&field("minimum_severity"))
            .and_then(Json::as_str);
        for rule in category.rules() {
            let id = rule.id();
            // A judgement rule has no finding to grade: a category entry never sets its
            // level, only a `rules` entry does (docs/contract/profiles.md).
            let categories: Vec<&str> = if rule.kind() == MatchKind::Judgement {
                Vec::new()
            } else {
                vec![qualified.as_str(), name]
            };
            let named = resolved.level_by_any_unfloored(&[id.as_str(), id.local()], &categories);
            // `severity_shift` moves levels that come from rule definitions, styles and the
            // genre chain. A level that the config sets is final. A category floor holds
            // after the shift.
            let level = match named {
                Some(named) => engine_level(named.level).map(|l| {
                    let l = if is_inherited(named.source.layer) {
                        shifted(l, shift)
                    } else {
                        l
                    };
                    if named.from == LevelFrom::Category {
                        floored(l, floor)
                    } else {
                        l
                    }
                }),
                // No layer names the rule: the engine shifts its genre or definition level.
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
    use slopvac_config::load;

    /// No packs behind the loader: these tests cover the settings builder, not packs.
    struct NoPresets;

    impl Presets for NoPresets {
        fn style(&self, _name: &str) -> Option<Vec<Setting>> {
            None
        }

        fn genre_chain(&self, _name: &str) -> Vec<Vec<Setting>> {
            Vec::new()
        }

        fn pack_config(&self, _name: &str) -> Option<PackFile> {
            None
        }
    }

    /// The settings that `config` (the text of a `slopvac.toml`) gives to a file.
    fn settings(config: &str) -> Result<Settings, CliError> {
        let rules = Catalog::load().expect("the rules load");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slopvac.toml");
        std::fs::write(&path, config).expect("write config");
        let loaded = load(&path, &NoPresets).expect("the config loads");
        validate(&loaded, &rules)?;
        let resolved = resolve(&loaded, Path::new("doc.md"));
        build(&rules, &resolved, Vec::new(), &Overrides::default())
    }

    fn level(settings: &Settings, rule: &str) -> Option<Level> {
        settings
            .rule_levels
            .get(&RuleId::new(rule).expect("a rule id"))
            .copied()
    }

    const HEAD: &str = "schema_version = 3\n";
    /// The 2.x `normal` levels apply only when the config lists the legacy style.
    const LEGACY: &str = "schema_version = 3\nstyles = [\"core:legacy-normal\"]\n";
    const COMMAND_PROMPT: &str = "core:prose-craft.command-prompt";
    const WARNING: Level = Level::On(Severity::Warning);
    const ERROR: Level = Level::On(Severity::Error);

    #[test]
    fn the_profile_turns_a_rule_off_and_a_category_entry_does_not_revive_it() {
        // The `normal` style sets `prose-craft.command-prompt` to off.
        assert_eq!(
            level(&settings(LEGACY).unwrap(), COMMAND_PROMPT),
            Some(Level::Off)
        );
        let config = format!("{LEGACY}\n[categories]\nprose-craft = \"warning\"\n");
        assert_eq!(
            level(&settings(&config).unwrap(), COMMAND_PROMPT),
            Some(Level::Off)
        );
    }

    #[test]
    fn without_the_legacy_style_a_rule_reports_at_its_definition_level() {
        // `prose-craft.command-prompt` ships as a warning. Only the legacy style turns it off.
        assert_eq!(level(&settings(HEAD).unwrap(), COMMAND_PROMPT), None);
        let listed = settings(LEGACY).unwrap();
        assert_eq!(level(&listed, COMMAND_PROMPT), Some(Level::Off));
    }

    #[test]
    fn a_rules_entry_revives_a_rule_the_profile_turns_off() {
        let config = format!("{LEGACY}\n[rules]\n\"{COMMAND_PROMPT}\" = \"warning\"\n");
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
        let base = settings(LEGACY).unwrap();
        assert_eq!(level(&base, rule), Some(Level::On(Severity::Suggestion)));
        let config =
            format!("{LEGACY}\n[categories.ste-punctuation]\nminimum_severity = \"error\"\n");
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

    /// `a.b` as an unknown rule at `slopvac.toml:4:1` in `layer`.
    fn unknown_in(layer: ConfigLayer) -> Vec<UnknownRule> {
        let mut setting = Setting::bare(layer, &["rules", "a.b"], Json::from("off"));
        setting.source = slopvac_config::located_source(layer, Some("slopvac.toml".into()), 4, 1);
        unknown_rules(&[setting], |_| false)
    }

    #[test]
    fn the_same_unknown_rule_at_one_source_reports_once() {
        let twice = [
            unknown_in(ConfigLayer::Extends),
            unknown_in(ConfigLayer::Extends),
        ]
        .concat();
        assert_eq!(without_repeats(twice).len(), 1);
    }

    #[test]
    fn the_same_position_in_two_layers_stays_two_findings() {
        let both = [
            unknown_in(ConfigLayer::Project),
            unknown_in(ConfigLayer::Override),
        ]
        .concat();
        assert_eq!(without_repeats(both).len(), 2);
    }

    #[test]
    fn an_unknown_category_is_a_config_error_naming_the_line() {
        let category = settings(&format!("{HEAD}\n[categories]\nnope = \"off\"\n"));
        let error = category.unwrap_err();
        assert_eq!(error.code, "config-unknown-category");
        assert!(error.message.contains("slopvac.toml:"), "{}", error.message);
    }

    #[test]
    fn an_unknown_rule_does_not_stop_the_settings_and_levels_the_known_rules_keep() {
        let config = format!(
            "{HEAD}\n[rules]\n\"orwell.nope\" = \"off\"\n\"{COMMAND_PROMPT}\" = \"warning\"\n"
        );
        assert_eq!(
            level(&settings(&config).unwrap(), COMMAND_PROMPT),
            Some(WARNING)
        );
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
            "{LEGACY}\n[profile]\nname = \"normal\"\nseverity_shift = 1\n\n[rules]\n\
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
        let down = format!("{LEGACY}\n[profile]\nname = \"normal\"\nseverity_shift = -1\n");
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

    #[test]
    fn a_qualified_and_a_bare_key_in_one_layer_resolve_the_same_in_either_order() {
        let rule = "core:prose-inflation.slop-lexicon";
        for entries in [
            "\"core:prose-inflation.slop-lexicon\" = \"off\"\n\"prose-inflation.slop-lexicon\" = \"error\"\n",
            "\"prose-inflation.slop-lexicon\" = \"error\"\n\"core:prose-inflation.slop-lexicon\" = \"off\"\n",
        ] {
            let s = settings(&format!("{HEAD}\n[rules]\n{entries}")).unwrap();
            // The qualified key wins on a tie, whatever the order it was written in.
            assert_eq!(level(&s, rule), Some(Level::Off), "{entries}");
        }
        for entries in [
            "\"core:orwell\" = \"off\"\norwell = \"error\"\n",
            "orwell = \"error\"\n\"core:orwell\" = \"off\"\n",
        ] {
            let s = settings(&format!("{HEAD}\n[categories]\n{entries}")).unwrap();
            assert_eq!(
                level(&s, "core:orwell.unsupported-evaluative"),
                Some(Level::Off),
                "{entries}"
            );
        }
    }

    #[test]
    fn a_none_budget_clears_the_budget_and_an_absent_one_keeps_the_rule_data() {
        let id = CategoryId::new("orwell").unwrap();
        let absent = settings(HEAD).unwrap();
        assert!(!absent.categories[&id].budget_cleared);
        let cleared = settings(&format!(
            "{HEAD}\n[categories.orwell]\nmax_per_100_words = \"none\"\n"
        ))
        .unwrap();
        assert!(cleared.categories[&id].budget_cleared);
        assert_eq!(cleared.categories[&id].max_per_100_words, None);
        let set = settings(&format!(
            "{HEAD}\n[categories.\"core:orwell\"]\nmax_per_100_words = 2.5\n"
        ))
        .unwrap();
        assert_eq!(set.categories[&id].max_per_100_words, Some(2.5));
        assert!(!set.categories[&id].budget_cleared);
    }

    #[test]
    fn the_category_floor_holds_after_the_severity_shift() {
        // `ste-punctuation` is a suggestion category. A shift of -1 cannot go lower, and the
        // floor of warning raises the shifted level.
        let config = format!(
            "{LEGACY}\n[profile]\nname = \"normal\"\nseverity_shift = -1\n\n\
             [categories.ste-punctuation]\nminimum_severity = \"warning\"\n"
        );
        let s = settings(&config).unwrap();
        assert_eq!(
            level(&s, "core:ste-punctuation.semicolon-used"),
            Some(WARNING)
        );
        // A category at error shifted down by one lands on warning, which the floor allows.
        let config = format!(
            "{LEGACY}\n[profile]\nname = \"normal\"\nseverity_shift = -1\n\n\
             [categories.orwell]\nminimum_severity = \"warning\"\n"
        );
        let s = settings(&config).unwrap();
        assert_eq!(
            level(&s, "core:orwell.unsupported-evaluative"),
            Some(WARNING)
        );
    }

    #[test]
    fn the_floor_applies_once_after_a_positive_shift() {
        // legacy-normal makes `ste-punctuation` a suggestion category. A shift of +1 raises
        // it to warning, and a floor of warning keeps it there. Flooring first would have
        // lifted it to warning and the shift on to error.
        let config = format!(
            "{LEGACY}\n[profile]\nname = \"normal\"\nseverity_shift = 1\n\n\
             [categories.ste-punctuation]\nminimum_severity = \"warning\"\n"
        );
        let s = settings(&config).unwrap();
        assert_eq!(
            level(&s, "core:ste-punctuation.semicolon-used"),
            Some(WARNING)
        );
    }

    #[test]
    fn a_nonzero_shift_leaves_unnamed_rules_to_the_engine() {
        // No layer names this rule, so the engine shifts its genre or definition level. An
        // entry here would outrank the genre levels.
        let config = format!("{HEAD}\n[profile]\nname = \"normal\"\nseverity_shift = 1\n");
        let s = settings(&config).unwrap();
        assert_eq!(s.severity_shift, 1);
        assert_eq!(
            level(&s, "core:docs-discipline.history-internal-reference"),
            None
        );
    }
}
