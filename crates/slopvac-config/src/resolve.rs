//! Merge the layers for one file and keep the history of every value.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value as Json, json};

use crate::load::{BlockKind, LoadedConfig};
use crate::presets::Presets;
use crate::source::{Layer, Setting, Source, display_path};

/// Built-in defaults: the lowest layer.
#[must_use]
pub fn core_defaults() -> Vec<Setting> {
    let core = |path: &[&str], value: Json| Setting::bare(Layer::Core, path, value);
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
    pub source: Source,
    /// The value.
    pub value: Json,
}

/// The merged settings of one file, with the history of each key.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    entries: BTreeMap<Vec<String>, Vec<Contribution>>,
}

fn is_sticky(path: &[String]) -> bool {
    path == ["network", "offline"]
}

impl Resolved {
    fn apply(&mut self, setting: Setting) {
        self.entries
            .entry(setting.path)
            .or_default()
            .push(Contribution {
                source: setting.source,
                value: setting.value,
            });
    }

    fn apply_all(&mut self, settings: impl IntoIterator<Item = Setting>) {
        for s in settings {
            self.apply(s);
        }
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
    pub fn source(&self, path: &[&str]) -> Option<&Source> {
        let (k, h) = self.lookup(path)?;
        Self::winner(k, h).map(|c| &c.source)
    }

    /// Every value set at `path`, lowest layer first.
    #[must_use]
    pub fn history(&self, path: &[&str]) -> &[Contribution] {
        self.lookup(path).map_or(&[], |(_, h)| h.as_slice())
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

/// One key in the `--explain-config` data.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Explained {
    /// The key path, such as `thresholds.min_score`.
    pub path: String,
    /// The final value.
    pub value: Json,
    /// Where the final value came from.
    pub source: Source,
    /// Lower contributions the final value replaced, lowest first.
    pub overridden: Vec<Contribution>,
}

impl LoadedConfig {
    /// Merge every layer for the file at `rel`, a path relative to the project root.
    #[must_use]
    pub fn resolve(&self, rel: &Path, inputs: &Inputs<'_>) -> Resolved {
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
        for name in styles.iter().filter_map(Json::as_str) {
            if let Some(settings) = inputs.presets.style(name) {
                out.apply_all(settings);
            }
        }

        let genre = inputs
            .front_matter
            .iter()
            .rfind(|s| s.path == ["genre"])
            .map(|s| &s.value)
            .or_else(|| {
                matching
                    .iter()
                    .rev()
                    .find(|b| b.kind == BlockKind::Genre)
                    .and_then(|b| b.settings.iter().find(|s| s.path == ["genre"]))
                    .map(|s| &s.value)
            })
            .and_then(Json::as_str);
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
pub fn describe(source: &Source) -> String {
    let mut out = source.layer.name().to_owned();
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
#[must_use]
pub fn render_explain(config: &LoadedConfig, explained: &[Explained]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "config root: {}", config.root.display());
    for f in &config.files {
        let _ = writeln!(out, "config file: {} ({})", f.display, f.layer.name());
    }
    for e in explained {
        let _ = writeln!(out, "{} = {}  [{}]", e.path, e.value, describe(&e.source));
        for o in e.overridden.iter().rev() {
            let _ = writeln!(out, "    replaced {}  [{}]", o.value, describe(&o.source));
        }
    }
    out
}
