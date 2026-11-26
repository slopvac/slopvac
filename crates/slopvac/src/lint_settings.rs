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
use slopvac_config::{
    ConfigLevel, GroupedKey, LevelFrom, LoadedConfig, Resolved, Setting, describe,
};
use slopvac_config::{Inputs, PackFile, Presets, UnknownRule, unknown_rules};
use slopvac_context::GenreAssignment;
use slopvac_engine::{CategoryDials, Level, Settings};
use slopvac_model::{
    CategoryName, ConfigLayer, ConfigSource, GenreId, GenreResult, GenreSource, KebabName,
    NonEmptyString, Notice, NoticeLevel, RESERVED_ID, RuleId, Severity,
};
use slopvac_rules::MatchKind;
use slopvac_styles::{BuiltinSource, Style, StylePresets};

use crate::errors::CliError;
use crate::lint_messages::LintOptionError;
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

const LEGACY: [(&str, &str); 6] = [
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
    (
        "legacy-strict-vocabulary",
        include_str!("../../../docs/contract/compat/profiles/legacy-strict-vocabulary.toml"),
    ),
    (
        "legacy-normal-vocabulary",
        include_str!("../../../docs/contract/compat/profiles/legacy-normal-vocabulary.toml"),
    ),
    (
        "legacy-relaxed-vocabulary",
        include_str!("../../../docs/contract/compat/profiles/legacy-relaxed-vocabulary.toml"),
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

fn legacy_style(name: &str) -> &'static Style {
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
    styles.get(name).unwrap_or_else(|| &styles["legacy-normal"])
}

/// The rule a config or command-line key names. `Ok(None)` when no rule has the key, and an
/// error when a bare key is the local id of rules in more than one plugin.
fn rule_id_of(rules: &Catalog, key: &str) -> Result<Option<RuleId>, LintOptionError> {
    match rules.rules_named(key).as_slice() {
        [] => Ok(None),
        [rule] => Ok(Some(rule.id().clone())),
        many => Err(LintOptionError::AmbiguousRule {
            name: format!("{key:?}"),
            candidates: many
                .iter()
                .map(|r| r.id().as_str())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

/// The category a config or command-line key names. `Ok(None)` when no plugin has the key,
/// and an error when a bare key is the id of categories in more than one plugin.
pub(crate) fn category_of(
    rules: &Catalog,
    key: &str,
) -> Result<Option<CategoryName>, LintOptionError> {
    match rules.categories_named(key).as_slice() {
        [] => Ok(None),
        [(plugin, category)] => Ok(Some(CategoryName::of(plugin, category.id()))),
        many => Err(LintOptionError::AmbiguousCategory {
            name: format!("{key:?}"),
            candidates: many
                .iter()
                .map(|(plugin, c)| format!("{plugin}:{}", c.id()))
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
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
        let located = |error| {
            Box::new(Located {
                error,
                source: Some(setting.source.clone()),
            })
        };
        match setting.path.as_slice() {
            [head, key, ..] if head == "categories" => match category_of(rules, key) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(located(CliError::escaping(
                        "config-unknown-category",
                        &format!(
                            "{}: unknown category {key:?}; use a category from `slopvac rules`",
                            describe(&redacted(&setting.source, root))
                        ),
                    )));
                }
                Err(ambiguous) => return Err(located(ambiguous.into())),
            },
            // An unknown rule is a notice, so only an ambiguous one stops the run.
            [head, key, ..] if head == "rules" => {
                if let Err(ambiguous) = rule_id_of(rules, key) {
                    return Err(located(ambiguous.into()));
                }
            }
            _ => {}
        }
    }
    unknown.extend(unknown_rules(settings, |key| {
        matches!(rule_id_of(rules, key), Ok(Some(_)))
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
                error: LintOptionError::UnknownStyle {
                    name: format!("{name:?}"),
                    shipped: shipped_styles(),
                }
                .into(),
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

/// The names of every style that ships, for the unknown-style message.
fn shipped_styles() -> String {
    BuiltinSource::names()
        .map(str::to_owned)
        .chain(LEGACY.iter().map(|(name, _)| format!("core:{name}")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The name a config lists to ask for the 2.x levels of a profile: `core:legacy-normal`,
/// `core:legacy-strict` or `core:legacy-relaxed`, or the vocabulary companion of one, such as
/// `core:legacy-normal-vocabulary`. `slopvac migrate --preserve-behavior` writes them. Nothing
/// applies them by default (docs/contract/profiles.md, section 1).
fn legacy_name(listed: &str) -> Option<&'static Style> {
    let name = listed.strip_prefix("core:")?;
    LEGACY
        .iter()
        .any(|(known, _)| *known == name)
        .then(|| legacy_style(name))
}

/// The built-in style presets, plus the `core:legacy-<profile>` styles.
pub(crate) struct ShippedStyles {
    styles: StylePresets<BuiltinSource>,
}

impl ShippedStyles {
    /// The styles and genres that ship with the binary.
    pub(crate) fn new() -> Self {
        Self {
            styles: StylePresets(BuiltinSource),
        }
    }
}

/// Qualify the bare `categories` and `rules` keys of a shipped layer to the plugin `core`,
/// which owns every shipped preset. A key written in a config file keeps its own spelling, so
/// the two never meet: a bare key that two plugins share stays ambiguous for a file and is
/// `core`'s for a preset.
fn qualified_to_core(layer: Vec<Setting>) -> Vec<Setting> {
    layer
        .into_iter()
        .map(|mut setting| {
            if let [head, key, ..] = setting.path.as_mut_slice()
                && (head == "categories" || head == "rules")
                && !key.contains(':')
            {
                *key = format!("{RESERVED_ID}:{key}");
            }
            setting
        })
        .collect()
}

impl Presets for ShippedStyles {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        match legacy_name(name) {
            Some(style) => Some(style.layer().to_vec()),
            None => self.styles.style(name),
        }
        .map(qualified_to_core)
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
        layers.into_iter().map(qualified_to_core).collect()
    }

    fn genre_chain(&self, name: &str) -> Vec<Vec<Setting>> {
        self.styles
            .genre_chain(name)
            .into_iter()
            .map(qualified_to_core)
            .collect()
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

/// Each key of `resolved` with its final value and the values it replaced, for
/// `--explain-config`. A `categories` or `rules` key is grouped under the catalogue entry it
/// names, so `x`, `core:x` and a shipped preset's `core:x` are one entry with one history. A
/// key that names no entry, or more than one, keeps its own spelling. The entry is shown under
/// the spelling of the contribution that won. A shipped preset's bare keys load as
/// `core:<key>` (see `qualified_to_core`), and are shown as the preset's author wrote them.
pub(crate) fn explain(resolved: &Resolved, catalog: &Catalog) -> Vec<slopvac_config::Explained> {
    resolved.explain_grouped(|path, contribution| {
        let mut shown = path.to_vec();
        let mut group = path.to_vec();
        let mut rank = 0;
        let mut fallback = None;
        if let [head, key, ..] = shown.as_mut_slice()
            && (head == "categories" || head == "rules")
        {
            rank = spelling_rank(key);
            // A preset key keeps the spelling its author wrote, whether or not it resolves.
            if is_inherited(contribution.source.layer)
                && let Some(bare) = key.strip_prefix(&format!("{RESERVED_ID}:"))
            {
                fallback = Some(path.to_vec());
                *key = bare.to_owned();
            }
            if let Some(identity) = identity_of(catalog, head, &path[1]) {
                group[1] = identity;
            }
        }
        let key = GroupedKey::new(group, shown, rank);
        match fallback {
            Some(fallback) => key.with_fallback(fallback),
            None => key,
        }
    })
}

/// The rank of a spelling of a `categories` or `rules` key when one layer writes a key more
/// than one way: the qualified spelling (0) wins over the bare one (1). `build` lists the
/// spellings it reads in this order, and `explain` ranks by it, so both report one value.
fn spelling_rank(key: &str) -> usize {
    usize::from(!key.contains(':'))
}

/// `spellings` in the order that wins a tie, by [`spelling_rank`].
fn by_rank<T>(mut spellings: Vec<(&str, T)>) -> Vec<T> {
    spellings.sort_by_key(|(key, _)| spelling_rank(key));
    spellings.into_iter().map(|(_, v)| v).collect()
}

/// The `<plugin>:<id>` of the one category or rule that `key` names under `section`.
fn identity_of(catalog: &Catalog, section: &str, key: &str) -> Option<String> {
    match section {
        "categories" => match catalog.categories_named(key).as_slice() {
            [(plugin, category)] => Some(format!("{}:{}", plugin.as_str(), category.id().as_str())),
            _ => None,
        },
        "rules" => match catalog.rules_named(key).as_slice() {
            [rule] => Some(rule.id().as_str().to_owned()),
            _ => None,
        },
        _ => None,
    }
}

/// The merged settings of the file at `rel`.
#[must_use]
pub fn resolve(config: &LoadedConfig, rel: &Path) -> Resolved {
    config.resolve(
        rel,
        &Inputs {
            presets: &ShippedStyles::new(),
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

/// What a `--category` or `--disable` name selects, with the plugin resolved.
enum Selected {
    /// A category, by the name a report gives it.
    Category(CategoryName),
    /// A rule.
    Rule(RuleId),
}

/// What a `--category` or `--disable` name selects: a category, or a rule in either form.
/// A qualified name keeps its plugin, and a bare name that two plugins share is an error.
fn selected(rules: &Catalog, names: &[String]) -> Result<Vec<Selected>, CliError> {
    let mut out = Vec::new();
    for name in names {
        if let Some(category) = category_of(rules, name)? {
            out.push(Selected::Category(category));
        } else if let Some(id) = rule_id_of(rules, name)? {
            out.push(Selected::Rule(id));
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

/// The settings of a file before any category or rule is set: profile, genre, thresholds and
/// the severity shift.
fn base_settings(
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
    Ok(settings)
}

/// The config keys that name one category: its qualified key, and the bare key when no other
/// plugin ships the same id.
struct CategoryKey<'a> {
    qualified: &'a str,
    bare: Option<&'a str>,
}

/// The level that the config gives one rule, and whether a config `rules` entry named it.
///
/// Only a `rules` entry from a config layer counts as naming the rule: a category entry, a
/// preset, a style and the genre chain turn rules on without naming them.
fn rule_level(
    rules: &Catalog,
    resolved: &Resolved,
    rule: &slopvac_rules::LoadedRule,
    category: &CategoryKey<'_>,
    shift: i64,
    floor: Option<&str>,
) -> (Option<Level>, bool) {
    let id = rule.id();
    // A judgement rule has no finding to grade: a category entry never sets its level, only
    // a `rules` entry does (docs/contract/profiles.md).
    let mut categories: Vec<(&str, &str)> = Vec::new();
    if rule.kind() != MatchKind::Judgement {
        categories.push((category.qualified, category.qualified));
        if let Some(bare) = category.bare {
            categories.push((bare, bare));
        }
    }
    let categories = by_rank(categories);
    let mut keys = vec![(id.as_str(), id.as_str())];
    if rules.rules_named(id.local()).len() == 1 {
        keys.push((id.local(), id.local()));
    }
    let keys = by_rank(keys);
    let Some(named) = resolved.level_by_any_unfloored(&keys, &categories) else {
        // No layer names the rule: the engine shifts its genre or definition level.
        return (None, false);
    };
    let by_config = named.from == LevelFrom::Rule && !is_inherited(named.source.layer);
    // `severity_shift` moves levels that come from rule definitions, styles and the genre
    // chain. A level that the config sets is final. A category floor holds after the shift.
    let level = engine_level(named.level).map(|l| {
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
    });
    (level, by_config && level.is_some())
}

/// Build the engine settings for a file from its resolved config.
///
/// `sources` is copied to each report document.
///
/// # Errors
/// `usage` for a `--category` or `--disable` name that the catalogue lacks, and
/// `ambiguous-category` or `ambiguous-rule` for a bare name that more than one plugin ships.
pub fn build(
    rules: &Catalog,
    resolved: &Resolved,
    sources: Vec<ConfigSource>,
    cli: &Overrides,
) -> Result<Settings, CliError> {
    let mut settings = base_settings(resolved, sources, cli)?;

    let shift = resolved
        .get(&["profile", "severity_shift"])
        .and_then(Json::as_i64)
        .unwrap_or(0);
    let disabled = selected(rules, &cli.disable)?;
    let only = selected(rules, &cli.only_categories)?;
    let categories = rules
        .sets()
        .iter()
        .flat_map(|set| set.categories().iter().map(move |c| (set.pack(), c)));
    for (plugin, category) in categories {
        let name = category.id().as_str();
        let qualified = format!("{plugin}:{name}");
        let reported = CategoryName::of(plugin, category.id());
        // A bare key names this category only when no other plugin ships the same id. The
        // shipped presets are qualified to `core` when they load.
        let bare_category = rules.categories_named(name).len() == 1;
        let field = |f: &'static str| {
            let mut paths = vec![(
                qualified.as_str(),
                vec!["categories", qualified.as_str(), f],
            )];
            if bare_category {
                paths.push((name, vec!["categories", name, f]));
            }
            by_rank(paths)
        };
        // `"none"` clears the budget; it is stored as null and must not fall back to the
        // budget in the rule data.
        let budget = resolved.get_by_any(&field("max_per_100_words"));
        settings.categories.insert(
            reported.clone(),
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
            let (level, named_by_config) = rule_level(
                rules,
                resolved,
                rule,
                &CategoryKey {
                    qualified: &qualified,
                    bare: bare_category.then_some(name),
                },
                shift,
                floor,
            );
            if let Some(level) = level {
                settings.rule_levels.insert(id.clone(), level);
            }
            let hit = |list: &[Selected]| {
                list.iter().any(|n| match n {
                    Selected::Category(c) => *c == reported,
                    Selected::Rule(r) => r == id,
                })
            };
            if hit(&disabled) || (!only.is_empty() && !hit(&only)) {
                settings.rule_levels.insert(id.clone(), Level::Off);
            }
            let named_by_only = only
                .iter()
                .any(|n| matches!(n, Selected::Rule(r) if r == id));
            let on = settings
                .rule_levels
                .get(id)
                .is_none_or(|l| *l != Level::Off);
            if on && (named_by_only || (named_by_config && level.is_some())) {
                settings.named_rules.insert(id.clone());
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
pub(crate) mod tests {
    use super::*;
    use crate::rule_catalog::{catalog_of, catalog_with_rules};
    use rstest::rstest;
    use serde_json::json;
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
        settings_over(&Catalog::load().expect("the rules load"), config)
    }

    /// The settings that `config` gives to a file, over the catalogue `rules`.
    fn settings_over(rules: &Catalog, config: &str) -> Result<Settings, CliError> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slopvac.toml");
        std::fs::write(&path, config).expect("write config");
        let loaded = load(&path, &NoPresets).expect("the config loads");
        validate(&loaded, rules)?;
        let resolved = resolve(&loaded, Path::new("doc.md"));
        build(rules, &resolved, Vec::new(), &Overrides::default())
    }

    /// A catalogue of two plugins, `alpha` and `beta`, that each ship a category `style`.
    fn two_plugins_with_one_category_id() -> Catalog {
        catalog_of(&[("alpha", "style"), ("beta", "style")])
    }

    /// The `explain` paths of `config` (the text of a `slopvac.toml`) over the catalogue `rules`.
    fn explained_paths(rules: &Catalog, config: &str) -> Vec<String> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slopvac.toml");
        std::fs::write(&path, config).expect("write config");
        let loaded = load(&path, &NoPresets).expect("the config loads");
        let resolved = resolve(&loaded, Path::new("doc.md"));
        explain(&resolved, rules)
            .into_iter()
            .map(|e| e.path)
            .filter(|p| p.starts_with("categories"))
            .collect()
    }

    #[test]
    fn explain_keeps_categories_of_two_plugins_with_one_local_id_apart() {
        let rules = catalog_of(&[("core", "style"), ("alpha", "style")]);
        let config =
            format!("{HEAD}\n[categories]\n\"core:style\" = \"off\"\n\"alpha:style\" = \"off\"\n");
        assert_eq!(
            explained_paths(&rules, &config),
            [
                "categories.\"alpha:style\".severity",
                "categories.\"core:style\".severity",
            ]
        );
    }

    // Both spellings in one layer: the qualified key wins the tie, as it does for `lint`
    // (`a_qualified_and_a_bare_key_in_one_layer_resolve_the_same_in_either_order`), so the one
    // entry shows the qualified spelling and its value. The bare value is the replaced one.
    #[test]
    fn explain_merges_the_bare_and_qualified_spellings_of_one_category() {
        let rules = catalog_of(&[("core", "style"), ("alpha", "tone")]);
        let config = format!("{HEAD}\n[categories]\n\"core:style\" = \"off\"\nstyle = \"error\"\n");
        assert_eq!(
            explained_paths(&rules, &config),
            ["categories.\"core:style\".severity"]
        );
    }

    /// `explain` over the shipped catalogue for `config`, with the shipped presets.
    fn explained_with_presets(config: &str) -> Vec<slopvac_config::Explained> {
        explained_with_presets_over(&Catalog::load().expect("the rules load"), config)
    }

    /// `explain` over `rules` for `config`, with the shipped presets.
    fn explained_with_presets_over(
        rules: &Catalog,
        config: &str,
    ) -> Vec<slopvac_config::Explained> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slopvac.toml");
        std::fs::write(&path, config).expect("write config");
        let loaded = load(&path, &ShippedStyles::new()).expect("the config loads");
        let resolved = resolve(&loaded, Path::new("doc.md"));
        explain(&resolved, rules)
    }

    // `explain` and `lint` read one value for a key that one layer spells both ways, in either
    // order, for a category and for a rule.
    #[rstest]
    #[case::category_qualified_first(
        "[categories]\n\"core:orwell\" = \"off\"\norwell = \"error\"\n",
        "categories.\"core:orwell\".severity",
        "core:orwell.unsupported-evaluative"
    )]
    #[case::category_bare_first(
        "[categories]\norwell = \"error\"\n\"core:orwell\" = \"off\"\n",
        "categories.\"core:orwell\".severity",
        "core:orwell.unsupported-evaluative"
    )]
    #[case::rule_qualified_first(
        "[rules]\n\"core:prose-inflation.slop-lexicon\" = \"off\"\n\"prose-inflation.slop-lexicon\" = \"error\"\n",
        "rules.\"core:prose-inflation.slop-lexicon\"",
        "core:prose-inflation.slop-lexicon"
    )]
    #[case::rule_bare_first(
        "[rules]\n\"prose-inflation.slop-lexicon\" = \"error\"\n\"core:prose-inflation.slop-lexicon\" = \"off\"\n",
        "rules.\"core:prose-inflation.slop-lexicon\"",
        "core:prose-inflation.slop-lexicon"
    )]
    fn explain_reports_the_value_that_lint_uses_for_a_key_spelled_both_ways(
        #[case] entries: &str,
        #[case] shown: &str,
        #[case] rule: &str,
    ) {
        let config = format!("{HEAD}\n{entries}");
        let explained = explained_with_presets(&config);
        let entry = explained
            .iter()
            .find(|e| e.path == shown)
            .unwrap_or_else(|| panic!("no entry {shown}: {explained:?}"));
        let s = settings(&config).unwrap();
        assert_eq!(level(&s, rule), Some(Level::Off));
        assert_eq!(entry.value, json!("off"));
    }

    // A plugin ships a category `ste-vocabulary`, and the project sets it by its bare name. The
    // `ste` preset's `ste-vocabulary` names no core category. Stripping the preset's `core:`
    // would print both under one path, so the preset's entry keeps the qualified spelling.
    #[test]
    fn a_preset_key_stays_qualified_when_its_bare_spelling_is_another_entries_path() {
        let rules = catalog_of(&[("core", "style"), ("alpha", "ste-vocabulary")]);
        let config =
            format!("{HEAD}styles = [\"ste\"]\n\n[categories]\nste-vocabulary = \"error\"\n");
        let explained = explained_with_presets_over(&rules, &config);
        let entries: Vec<(&str, &serde_json::Value)> = explained
            .iter()
            .filter(|e| e.path.contains("ste-vocabulary"))
            .map(|e| (e.path.as_str(), &e.value))
            .collect();
        assert_eq!(
            entries,
            [
                ("categories.ste-vocabulary.severity", &json!("error")),
                ("categories.\"core:ste-vocabulary\".severity", &json!("off")),
            ]
        );
    }

    // A preset key keeps the spelling its author wrote, whether or not it names a catalogue
    // entry. `ste-vocabulary` names no category.
    #[test]
    fn a_preset_key_keeps_its_own_spelling_when_it_names_no_catalogue_entry() {
        let explained = explained_with_presets(&format!("{HEAD}styles = [\"ste\"]\n"));
        let paths: Vec<&str> = explained
            .iter()
            .map(|e| e.path.as_str())
            .filter(|p| p.contains("ste-vocabulary"))
            .collect();
        assert_eq!(paths, ["categories.ste-vocabulary.severity"]);
    }

    /// The settings that the command-line `cli` gives over `rules` and an empty config.
    fn settings_with(rules: &Catalog, cli: &Overrides) -> Result<Settings, CliError> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slopvac.toml");
        std::fs::write(&path, HEAD).expect("write config");
        let loaded = load(&path, &NoPresets).expect("the config loads");
        let resolved = resolve(&loaded, Path::new("doc.md"));
        build(rules, &resolved, Vec::new(), cli)
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

    #[test]
    fn a_config_entry_for_one_plugins_category_leaves_the_other_plugins_category_alone() {
        let rules = two_plugins_with_one_category_id();
        let config = format!("{HEAD}\n[categories.\"alpha:style\"]\nweight = 0.0\n");
        let s = settings_over(&rules, &config).unwrap();
        let dials: Vec<(&str, Option<f64>)> = s
            .categories
            .iter()
            .map(|(name, d)| (name.as_str(), d.weight))
            .collect();
        assert_eq!(dials, [("alpha:style", Some(0.0)), ("beta:style", None)]);
    }

    #[rstest]
    #[case::category("[categories.style]\nweight = 0.0\n", "ambiguous-category")]
    #[case::rule("[rules]\n\"style.hit\" = \"off\"\n", "ambiguous-rule")]
    fn a_bare_config_key_that_two_plugins_share_is_refused(
        #[case] table: &str,
        #[case] code: &str,
    ) {
        let rules = two_plugins_with_one_category_id();
        let error = settings_over(&rules, &format!("{HEAD}\n{table}")).unwrap_err();
        assert_eq!(error.code, code);
        assert!(error.message.contains("alpha:style"), "{}", error.message);
        assert!(error.message.contains("beta:style"), "{}", error.message);
    }

    #[test]
    fn a_bare_category_key_that_one_plugin_has_applies_to_that_plugin() {
        let rules = catalog_of(&[("alpha", "style"), ("beta", "tone")]);
        let config = format!("{HEAD}\n[categories.style]\nweight = 0.0\n");
        let s = settings_over(&rules, &config).unwrap();
        let dials: Vec<(&str, Option<f64>)> = s
            .categories
            .iter()
            .map(|(name, d)| (name.as_str(), d.weight))
            .collect();
        assert_eq!(dials, [("alpha:style", Some(0.0)), ("beta:tone", None)]);
    }

    #[rstest]
    #[case::a_category("alpha:style")]
    #[case::a_rule("alpha:style.hit")]
    fn disabling_a_qualified_name_leaves_the_other_plugin_alone(#[case] name: &str) {
        let rules = two_plugins_with_one_category_id();
        let cli = Overrides {
            disable: vec![name.to_owned()],
            ..Overrides::default()
        };
        let s = settings_with(&rules, &cli).unwrap();
        assert_eq!(level(&s, "alpha:style.hit"), Some(Level::Off));
        assert_eq!(level(&s, "beta:style.hit"), None);
    }

    #[test]
    fn selecting_a_qualified_category_runs_only_that_plugins_category() {
        let rules = two_plugins_with_one_category_id();
        let cli = Overrides {
            only_categories: vec!["alpha:style".to_owned()],
            ..Overrides::default()
        };
        let s = settings_with(&rules, &cli).unwrap();
        assert_eq!(level(&s, "alpha:style.hit"), None);
        assert_eq!(level(&s, "beta:style.hit"), Some(Level::Off));
    }

    #[rstest]
    #[case::disable("style", true, "ambiguous-category")]
    #[case::select("style", false, "ambiguous-category")]
    #[case::disable_rule("style.hit", true, "ambiguous-rule")]
    fn a_bare_command_line_name_that_two_plugins_share_is_refused(
        #[case] name: &str,
        #[case] disable: bool,
        #[case] code: &str,
    ) {
        let rules = two_plugins_with_one_category_id();
        let mut cli = Overrides::default();
        let list = if disable {
            &mut cli.disable
        } else {
            &mut cli.only_categories
        };
        list.push(name.to_owned());
        assert_eq!(settings_with(&rules, &cli).unwrap_err().code, code);
    }

    #[test]
    fn a_shipped_preset_dial_stays_with_core_when_a_plugin_shares_the_category_id() {
        // The shipped `ste` style gives `ste-words` a weight of 0.8 by its bare name.
        let mut sets = Catalog::load().expect("the rules load").into_sets();
        sets.extend(catalog_of(&[("alpha", "ste-words")]).into_sets());
        let rules = Catalog::from_sets(sets);
        let config = format!("{HEAD}styles = [\"ste\"]\n");
        let s = settings_over(&rules, &config).unwrap();
        let weight = |name: &str| s.categories[&CategoryName::new(name).unwrap()].weight;
        assert_eq!(weight("ste-words"), Some(0.8));
        assert_eq!(weight("alpha:ste-words"), None);
    }

    /// The catalogue of the shipped rules and a plugin `alpha` that ships a rule with the same
    /// local id as the core rule `ste-practices.latin-abbreviation`, which the `ste` style
    /// turns off by its bare name.
    pub(crate) fn core_and_a_plugin_sharing_a_preset_rule() -> Catalog {
        let mut sets = Catalog::load().expect("the rules load").into_sets();
        sets.extend(
            catalog_with_rules(&[("alpha", "ste-practices", "latin-abbreviation")]).into_sets(),
        );
        Catalog::from_sets(sets)
    }

    #[test]
    fn a_shipped_preset_rule_level_stays_with_core_when_a_plugin_shares_the_rule_id() {
        let rules = core_and_a_plugin_sharing_a_preset_rule();
        let s = settings_over(&rules, &format!("{HEAD}styles = [\"ste\"]\n")).unwrap();
        assert_eq!(
            level(&s, "core:ste-practices.latin-abbreviation"),
            Some(Level::Off)
        );
        assert_eq!(level(&s, "alpha:ste-practices.latin-abbreviation"), None);
    }

    #[test]
    fn an_ambiguous_bare_key_never_reaches_core_when_a_config_file_wrote_it() {
        // `validate` refuses the key, so the settings builder must not use it either.
        let rules = core_and_a_plugin_sharing_a_preset_rule();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slopvac.toml");
        let text = format!("{HEAD}[rules]\n\"ste-practices.latin-abbreviation\" = \"error\"\n");
        std::fs::write(&path, text).expect("write config");
        let loaded = load(&path, &NoPresets).expect("the config loads");
        let resolved = resolve(&loaded, Path::new("doc.md"));
        let s = build(&rules, &resolved, Vec::new(), &Overrides::default()).unwrap();
        assert_eq!(level(&s, "core:ste-practices.latin-abbreviation"), None);
        assert_eq!(level(&s, "alpha:ste-practices.latin-abbreviation"), None);
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
        let id = CategoryName::new("orwell").unwrap();
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

    /// The rules that the settings of `config` mark as named by the user.
    fn named(config: &str, overrides: &Overrides) -> Vec<String> {
        let rules = Catalog::load().expect("the rules load");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("slopvac.toml");
        std::fs::write(&path, config).expect("write config");
        let loaded = load(&path, &NoPresets).expect("the config loads");
        validate(&loaded, &rules).expect("the config validates");
        let resolved = resolve(&loaded, Path::new("doc.md"));
        let settings = build(&rules, &resolved, Vec::new(), overrides).expect("settings");
        settings
            .named_rules
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect()
    }

    #[rstest::rstest]
    #[case::rules_entry("[rules]\n\"core:prose-craft.sentence-length\" = \"warning\"\n", &["core:prose-craft.sentence-length"])]
    #[case::bare_rules_entry("[rules]\n\"prose-craft.sentence-length\" = \"warning\"\n", &["core:prose-craft.sentence-length"])]
    #[case::rules_entry_off("[rules]\n\"core:prose-craft.sentence-length\" = \"off\"\n", &[])]
    #[case::category_entry("[categories]\n\"core:prose-craft\" = \"warning\"\n", &[])]
    #[case::style_and_genre_only("styles = [\"core:legacy-normal\"]\n", &[])]
    fn only_a_rules_entry_names_a_rule(#[case] body: &str, #[case] expected: &[&str]) {
        assert_eq!(
            named(&format!("{HEAD}{body}"), &Overrides::default()),
            expected
        );
    }

    #[test]
    fn a_rule_selected_by_id_on_the_command_line_is_named_and_a_disabled_one_is_not() {
        let rule = "core:prose-craft.sentence-length".to_owned();
        let only = Overrides {
            only_categories: vec![rule.clone()],
            ..Overrides::default()
        };
        assert_eq!(named(HEAD, &only), std::slice::from_ref(&rule));
        let disabled = Overrides {
            disable: vec![rule],
            ..Overrides::default()
        };
        assert_eq!(named(HEAD, &disabled), Vec::<String>::new());
    }
}
