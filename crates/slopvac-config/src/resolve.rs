//! Merge the layers for one file and keep the history of every value.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value as Json, json};
use slopvac_model::{ConfigLayer, ConfigSource, Severity};

use crate::load::{BlockKind, LoadedConfig};
use crate::messages::ConfigText;
use crate::presets::Presets;
use crate::source::{Setting, display_path, layer_name};

/// Built-in defaults: the lowest layer.
#[must_use]
pub fn core_defaults() -> Vec<Setting> {
    let core = |path: &[&str], value: Json| Setting::bare(ConfigLayer::Core, path, value);
    vec![
        core(&["mode"], json!("prose")),
        core(
            &["exclude"],
            json!([
                "**/node_modules/**",
                "**/apm_modules/**",
                "**/.venv/**",
                "**/dist/**",
                "**/build/**",
                "**/CHANGELOG.md",
                "**/.git/**"
            ]),
        ),
        core(&["profile", "name"], json!("normal")),
        core(&["frontmatter", "enabled"], json!(true)),
        core(
            &["frontmatter", "allow"],
            json!(["genre", "locale", "terms"]),
        ),
        core(&["guidance", "level"], json!("standard")),
        core(&["guidance", "write"], json!("auto")),
        core(&["guidance", "review"], json!("auto")),
        core(&["guidance", "review_depth"], json!("subagent")),
        core(&["guidance", "report"], json!("ask")),
        core(&["network", "offline"], json!(false)),
        core(&["network", "update_check"], json!(true)),
    ]
}

/// Settings that come from outside the config files, for one document.
#[derive(Clone, Copy)]
pub struct Inputs<'a> {
    /// Where styles, genres and pack configs come from.
    pub presets: &'a dyn Presets,
    /// Settings from the document's `slopvac:` block, already limited by the allow policy.
    pub front_matter: &'a [Setting],
    /// Settings from inline directives in the document.
    pub directives: &'a [Setting],
    /// Settings from command-line flags.
    pub cli: &'a [Setting],
    /// Settings from environment switches.
    pub env: &'a [Setting],
}

/// A value one layer set.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Contribution {
    /// Where it came from.
    pub source: ConfigSource,
    /// The value. A category budget of `"none"` is stored as `null`.
    pub value: Json,
    /// Which layer instance set it: a style, a genre layer, a config file, a block, and so on,
    /// numbered in the order they apply. Level resolution compares these.
    #[serde(skip)]
    scope: usize,
}

/// The merged settings of one file, with the history of each key.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    entries: BTreeMap<Vec<String>, Vec<Contribution>>,
    scopes: usize,
}

fn is_sticky(path: &[String]) -> bool {
    path == ["network", "offline"]
}

impl Resolved {
    fn apply(&mut self, setting: Setting, scope: usize) {
        let clears_budget = matches!(
            setting.path.as_slice(),
            [c, _, f] if c == "categories" && f == "max_per_100_words"
        ) && setting.value == json!("none");
        let value = if clears_budget {
            Json::Null
        } else {
            setting.value
        };
        self.entries
            .entry(setting.path)
            .or_default()
            .push(Contribution {
                source: setting.source,
                value,
                scope,
            });
    }

    /// Apply the settings of one layer instance. A later instance outranks an earlier one.
    fn apply_all(&mut self, settings: impl IntoIterator<Item = Setting>) {
        let scope = self.scopes;
        self.scopes += 1;
        for s in settings {
            self.apply(s, scope);
        }
    }

    /// Merge `layers`, lowest first, with no core defaults. Each layer is one scope, so a
    /// later layer outranks an earlier one. Use it to resolve a part of the stack, such as the
    /// style layers alone.
    #[must_use]
    pub fn from_layers(layers: impl IntoIterator<Item = Vec<Setting>>) -> Self {
        let mut out = Self::default();
        for layer in layers {
            out.apply_all(layer);
        }
        out
    }

    fn winner<'a>(path: &[String], history: &'a [Contribution]) -> Option<&'a Contribution> {
        if is_sticky(path)
            && let Some(on) = history.iter().find(|c| c.value == json!(true))
        {
            return Some(on);
        }
        history.last()
    }

    fn lookup(&self, path: &[&str]) -> Option<(&Vec<String>, &Vec<Contribution>)> {
        self.entries
            .iter()
            .find(|(k, _)| k.iter().map(String::as_str).eq(path.iter().copied()))
    }

    /// The final value at `path`.
    #[must_use]
    pub fn get(&self, path: &[&str]) -> Option<&Json> {
        let (k, h) = self.lookup(path)?;
        Self::winner(k, h).map(|c| &c.value)
    }

    /// The layer and location that set the final value at `path`.
    #[must_use]
    pub fn source(&self, path: &[&str]) -> Option<&ConfigSource> {
        let (k, h) = self.lookup(path)?;
        Self::winner(k, h).map(|c| &c.source)
    }

    /// Every value set at `path`, lowest layer first.
    #[must_use]
    pub fn history(&self, path: &[&str]) -> &[Contribution] {
        self.lookup(path).map_or(&[], |(_, h)| h.as_slice())
    }

    /// The density budget of a category, or `None` when no layer sets one or a higher layer
    /// cleared it with `"none"`.
    #[must_use]
    pub fn budget(&self, category: &str) -> Option<f64> {
        self.get(&["categories", category, "max_per_100_words"])
            .and_then(Json::as_f64)
    }

    /// The level a rule reports at, by the precedence in `docs/contract/profiles.md`, section 3.
    ///
    /// The highest layer that names the rule or its category wins, and within one layer a
    /// `rules` entry beats a `categories` entry. A `rules` entry at `off` holds against every
    /// category entry, and only a higher `rules` entry revives the rule. A category level rises
    /// to the category's merged `minimum_severity`, where `off` sets no floor. `None` means no
    /// layer names the rule, so it reports at the level of its definition.
    #[must_use]
    pub fn level(&self, rule: &str, category: Option<&str>) -> Option<ResolvedLevel> {
        self.level_by_any(&[rule], category.as_slice())
    }

    /// [`Resolved::level`] for a rule and a category that a file may spell more than one way,
    /// such as `core:r` and `r`. Every spelling in `rules` and `categories` is read, and the
    /// layer that wrote an entry decides which one wins, not the spelling of its key. On a tie
    /// the earlier spelling in the slice wins.
    #[must_use]
    pub fn level_by_any(&self, rules: &[&str], categories: &[&str]) -> Option<ResolvedLevel> {
        self.pick_level(rules, categories, true)
    }

    /// [`Resolved::level_by_any`] without the `minimum_severity` floor. A caller that moves
    /// the level itself, such as by a profile's `severity_shift`, applies the floor once after.
    #[must_use]
    pub fn level_by_any_unfloored(
        &self,
        rules: &[&str],
        categories: &[&str],
    ) -> Option<ResolvedLevel> {
        self.pick_level(rules, categories, false)
    }

    fn pick_level(
        &self,
        rules: &[&str],
        categories: &[&str],
        floor: bool,
    ) -> Option<ResolvedLevel> {
        let floor = |level: ResolvedLevel| {
            if floor {
                self.floored(level, categories)
            } else {
                level
            }
        };
        let by_rule = self.newest(rules.iter().map(|r| vec!["rules", *r]));
        let by_category = self.newest(
            categories
                .iter()
                .map(|c| vec!["categories", *c, "severity"]),
        );
        let as_level = |c: &Contribution, from| {
            Some(ResolvedLevel {
                level: ConfigLevel::from_value(&c.value)?,
                source: c.source.clone(),
                from,
            })
        };
        match (by_rule, by_category) {
            (Some(r), Some(c)) if r.value != json!("off") && c.scope > r.scope => {
                Some(floor(as_level(c, LevelFrom::Category)?))
            }
            (Some(r), _) => as_level(r, LevelFrom::Rule),
            (None, Some(c)) => Some(floor(as_level(c, LevelFrom::Category)?)),
            (None, None) => None,
        }
    }

    /// The contribution from the highest layer among the entries at `paths`.
    fn newest<'a>(
        &'a self,
        paths: impl IntoIterator<Item = Vec<&'a str>>,
    ) -> Option<&'a Contribution> {
        let mut best: Option<&Contribution> = None;
        for path in paths {
            let Some(found) = self.lookup(&path).and_then(|(_, h)| h.last()) else {
                continue;
            };
            if best.is_none_or(|b| found.scope > b.scope) {
                best = Some(found);
            }
        }
        best
    }

    /// The final value among the entries at `paths`, taken from the highest layer that wrote
    /// one, for a key that a file may spell more than one way. A cleared budget is `null`.
    #[must_use]
    pub fn get_by_any<'a>(&'a self, paths: &[Vec<&'a str>]) -> Option<&'a Json> {
        self.newest(paths.iter().cloned()).map(|c| &c.value)
    }

    fn floored(&self, mut level: ResolvedLevel, categories: &[&str]) -> ResolvedLevel {
        let floor = self
            .newest(
                categories
                    .iter()
                    .map(|c| vec!["categories", *c, "minimum_severity"]),
            )
            .and_then(|c| ConfigLevel::from_value(&c.value));
        // `off` as a floor sets none, and an `off` level is never raised.
        if let (Some(ConfigLevel::On(min)), ConfigLevel::On(have)) = (floor, level.level)
            && have < min
        {
            level.level = ConfigLevel::On(min);
        }
        level
    }

    /// Whether the run is offline. Once any layer sets it, no layer can unset it.
    #[must_use]
    pub fn is_offline(&self) -> bool {
        self.get(&["network", "offline"]) == Some(&json!(true))
    }

    /// The merged settings as a nested JSON object.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut root = Map::new();
        for (path, history) in &self.entries {
            if let Some(win) = Self::winner(path, history) {
                insert_at(&mut root, path, win.value.clone());
            }
        }
        Json::Object(root)
    }

    /// Each key with its final value, where it came from, and the values it replaced.
    #[must_use]
    pub fn explain(&self) -> Vec<Explained> {
        self.entries
            .iter()
            .filter_map(|(path, history)| {
                let win = Self::winner(path, history)?;
                Some(Explained {
                    path: display_path(path),
                    value: win.value.clone(),
                    source: win.source.clone(),
                    overridden: history
                        .iter()
                        .filter(|c| !std::ptr::eq(*c, win))
                        .cloned()
                        .collect(),
                })
            })
            .collect()
    }
}

fn insert_at(map: &mut Map<String, Json>, path: &[String], value: Json) {
    match path {
        [] => {}
        [last] => {
            map.insert(last.clone(), value);
        }
        [head, rest @ ..] => {
            let slot = map
                .entry(head.clone())
                .or_insert_with(|| Json::Object(Map::new()));
            if !slot.is_object() {
                *slot = Json::Object(Map::new());
            }
            if let Json::Object(inner) = slot {
                insert_at(inner, rest, value);
            }
        }
    }
}

/// A level as config spells it: a severity, or `off`, which `Severity` has no value for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigLevel {
    /// The rule does not run.
    Off,
    /// The rule reports at this severity.
    On(Severity),
}

impl ConfigLevel {
    /// The level a config value names, or `None` when it names neither `off` nor a severity.
    #[must_use]
    pub fn from_value(value: &Json) -> Option<Self> {
        if value == "off" {
            return Some(Self::Off);
        }
        serde_json::from_value(value.clone()).ok().map(Self::On)
    }

    /// The config spelling: `off`, `suggestion`, `warning` or `error`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On(Severity::Suggestion) => "suggestion",
            Self::On(Severity::Warning) => "warning",
            Self::On(Severity::Error) => "error",
        }
    }
}

/// Which entry set a resolved level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelFrom {
    /// A `rules` entry.
    Rule,
    /// A `categories` entry.
    Category,
}

/// The level of one rule after layer precedence.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLevel {
    /// The severity, or `off`.
    pub level: ConfigLevel,
    /// The layer and location of the entry that set it.
    pub source: ConfigSource,
    /// Whether a rule entry or a category entry set it.
    pub from: LevelFrom,
}

/// One key in the `--explain-config` data.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Explained {
    /// The key path, such as `thresholds.min_score`.
    pub path: String,
    /// The final value.
    pub value: Json,
    /// Where the final value came from.
    pub source: ConfigSource,
    /// Lower contributions the final value replaced, lowest first.
    pub overridden: Vec<Contribution>,
}

impl LoadedConfig {
    /// Merge every layer for the file at `rel`, a path relative to the project root.
    #[must_use]
    pub fn resolve(&self, rel: &Path, inputs: &Inputs<'_>) -> Resolved {
        self.resolve_detecting(rel, inputs, None)
    }

    /// As [`LoadedConfig::resolve`], with the genre that detection chose for the document.
    /// It supplies the genre layer only when no `[[genres]]` block, front matter, directive
    /// or flag names a genre: an explicit genre always wins and the layer then comes from it.
    #[must_use]
    pub fn resolve_detecting(
        &self,
        rel: &Path,
        inputs: &Inputs<'_>,
        detected: Option<&str>,
    ) -> Resolved {
        let mut out = Resolved::default();
        out.apply_all(core_defaults());

        let matching: Vec<_> = self
            .files
            .iter()
            .flat_map(|f| f.blocks.iter())
            .filter(|b| b.matches(rel))
            .collect();

        let styles = self
            .project_value(&["styles"])
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let listed: Vec<String> = styles
            .iter()
            .filter_map(Json::as_str)
            .map(str::to_owned)
            .collect();
        for settings in inputs.presets.style_layers(&listed) {
            out.apply_all(settings);
        }

        // The genre can come from any layer above the genre layer, so pick the winner first.
        let genre_blocks = matching
            .iter()
            .filter(|b| b.kind == BlockKind::Genre)
            .flat_map(|b| b.settings.iter());
        let genre = genre_blocks
            .chain(inputs.front_matter)
            .chain(inputs.directives)
            .chain(inputs.cli)
            .rfind(|s| s.path == ["genre"])
            .and_then(|s| s.value.as_str())
            .or(detected);
        if let Some(name) = genre {
            for layer in inputs.presets.genre_chain(name) {
                out.apply_all(layer);
            }
        }

        for file in &self.files {
            out.apply_all(file.settings.iter().cloned());
        }
        for block in matching {
            out.apply_all(block.settings.iter().cloned());
        }
        out.apply_all(inputs.front_matter.iter().cloned());
        out.apply_all(inputs.directives.iter().cloned());
        out.apply_all(inputs.cli.iter().cloned());
        out.apply_all(inputs.env.iter().cloned());
        out
    }
}

/// Where a source points, such as `project slopvac.toml:3:1` or `core`.
#[must_use]
pub fn describe(source: &ConfigSource) -> String {
    let mut out = layer_name(source.layer).to_owned();
    if let Some(file) = &source.file {
        out.push(' ');
        out.push_str(file);
        if let (Some(line), Some(col)) = (source.line, source.column) {
            let _ = write!(out, ":{line}:{col}");
        }
    }
    out
}

/// The `--explain-config` text: each key, its final value and origin, then the values it replaced.
///
/// `root` is how the config root appears in the text. The caller redacts it (policies 1.7), so
/// the root and the path metadata (the config files and the origins) carry no absolute path.
/// Values print as configured. Every field passes through
/// [`slopvac_sanitize::line`], so a file name or a value cannot inject a control sequence or a
/// line break.
#[must_use]
pub fn render_explain(config: &LoadedConfig, explained: &[Explained], root: &str) -> String {
    use slopvac_sanitize::line;
    let mut out = String::new();
    let root = ConfigText::ExplainRoot { root: line(root) };
    let _ = writeln!(out, "{root}");
    for f in &config.files {
        let file = ConfigText::ExplainFile {
            file: line(&f.display),
            layer: line(layer_name(f.layer)),
        };
        let _ = writeln!(out, "{file}");
    }
    for e in explained {
        let setting = ConfigText::ExplainSetting {
            path: line(&e.path),
            value: line(&e.value.to_string()),
            origin: line(&describe(&e.source)),
        };
        let _ = writeln!(out, "{setting}");
        for o in e.overridden.iter().rev() {
            let replaced = ConfigText::ExplainReplaced {
                value: line(&o.value.to_string()),
                origin: line(&describe(&o.source)),
            };
            let _ = writeln!(out, "    {replaced}");
        }
    }
    out
}
