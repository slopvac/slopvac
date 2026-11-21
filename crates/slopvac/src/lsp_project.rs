//! The language server's view of the project: the settings of each open document, and the
//! reload when a config file changes.
//!
//! A document gets the settings `slopvac lint` would give it: the nearest `slopvac.toml`
//! of its directory, with the `[[overrides]]` and `[[genres]]` blocks that match its path
//! (see [`crate::engine::settings_for`]). One engine serves every document whose settings
//! are equal, as in a lint run.
//!
//! A reload throws away every loaded config and engine, reads the project config again and
//! reports what it found. A config that cannot be read leaves no document looking checked:
//! every document is reported as not linted, with the config error as the reason, until a
//! later reload succeeds. The earlier settings are not kept, because a result computed from
//! them would pass for a result of the config on disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use slopvac_config::frontmatter::Policy;
use slopvac_config::{ConfigError, Discovery, LoadedConfig};
use slopvac_lsp::{
    Check, ConfigFailure, EngineLinter, LintError, Linted, Linter, Problem, Reloaded,
};
use slopvac_model::{Notice, RuleId};
use slopvac_styles::{BuiltinSource, StylePresets};

use crate::engine::{config_error, declared_genre_of, rel_text, settings_for};
use crate::errors::CliError;
use crate::lint_settings::{self, Overrides};
use crate::lint_terms;
use crate::rule_catalog::Catalog;

/// What the language server needs from the project: the linter, the config warnings and the
/// extra checks it runs next to the engine. Term casing comes from the engine.
pub(crate) struct ProjectServer {
    /// Lints each document with its own settings, and reloads on request.
    pub linter: ProjectLinter,
    /// Warnings about the config.
    pub notices: Vec<Notice>,
    /// Front-matter validation.
    pub checks: Vec<Arc<dyn Check>>,
}

/// Why the project config could not be loaded, with where to show it.
struct LoadFailure {
    error: CliError,
    /// Absolute path of the file that failed.
    path: PathBuf,
    /// Zero-based line and column in that file.
    position: Option<(u32, u32)>,
}

/// What a load of the project produced. A reload replaces all of it.
#[derive(Default)]
struct Loaded {
    /// Set when the project config could not be loaded.
    failure: Option<String>,
    catalog: Option<Catalog>,
    /// The nearest config of each directory, as `slopvac lint` finds it.
    configs: HashMap<PathBuf, Option<Arc<LoadedConfig>>>,
    /// One engine for each distinct settings set.
    engines: HashMap<String, Arc<EngineLinter>>,
    /// Absolute paths of the config files of the project.
    files: Vec<String>,
}

struct Project {
    root: PathBuf,
    loaded: Mutex<Loaded>,
}

/// A document's engine and what the engine needs to know about its place in the project.
struct Resolved {
    engine: Arc<EngineLinter>,
    relative: Option<String>,
    genre: Option<String>,
}

/// [`Linter`] over the whole project: it picks the engine of each document.
pub(crate) struct ProjectLinter {
    project: Arc<Project>,
}

/// Front-matter validation under the config of each document.
struct ProjectCheck {
    project: Arc<Project>,
}

/// The language server's linter for the project config found from `root`: the shipped
/// rules, the term-casing pack, the settings, the project's terms files and the blocklists,
/// built the same way `slopvac lint` builds an engine. The notices are the warnings about the
/// config, which the language server shows to the client.
///
/// # Errors
/// The config error, `terms-invalid`, or `rules-unavailable`.
pub(crate) fn project_linter(root: &Path) -> Result<ProjectServer, CliError> {
    let project = Arc::new(Project {
        root: root.to_path_buf(),
        loaded: Mutex::new(Loaded::default()),
    });
    let notices = project.load().map_err(|failure| failure.error)?;
    Ok(ProjectServer {
        linter: ProjectLinter {
            project: Arc::clone(&project),
        },
        notices,
        checks: vec![Arc::new(ProjectCheck { project })],
    })
}

/// The nearest config of `dir`, found again from disk.
fn nearest(dir: &Path) -> Result<Option<Arc<LoadedConfig>>, ConfigError> {
    let presets = StylePresets(BuiltinSource);
    Discovery::new(&presets).nearest(dir)
}

impl Project {
    fn loaded(&self) -> MutexGuard<'_, Loaded> {
        self.loaded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replace everything with a fresh read of the project config.
    fn load(&self) -> Result<Vec<Notice>, Box<LoadFailure>> {
        let mut fresh = Loaded::default();
        let result = self.read_into(&mut fresh);
        if let Err(failure) = &result {
            fresh.failure = Some(failure.error.message.clone());
        }
        *self.loaded() = fresh;
        result
    }

    fn read_into(&self, loaded: &mut Loaded) -> Result<Vec<Notice>, Box<LoadFailure>> {
        let fallback = |error: CliError| {
            Box::new(LoadFailure {
                error,
                path: self.root.join("slopvac.toml"),
                position: None,
            })
        };
        let core = Catalog::load().map_err(fallback)?;
        let config = nearest(&self.root).map_err(|e| Box::new(self.failure_of(&e)))?;
        let notices = match &config {
            Some(config) => lint_settings::validate(config, &core).map_err(fallback)?,
            None => Vec::new(),
        };
        if let Some(config) = &config {
            loaded.files = config
                .files
                .iter()
                .filter(|f| !f.display.starts_with('<'))
                .map(|f| config.root.join(&f.display).display().to_string())
                .collect();
        }
        loaded.configs.insert(self.root.clone(), config);
        loaded.catalog = Some(core);
        Ok(notices)
    }

    /// The nearest config of `dir`, found again from disk.
    fn failure_of(&self, err: &ConfigError) -> LoadFailure {
        let (path, position) = match err {
            ConfigError::Rejected(rejected) => (
                rejected.file.clone(),
                rejected.diagnostics.first().map(|d| {
                    let (line, column) = d.location(&rejected.text);
                    (
                        u32::try_from(line.saturating_sub(1)).unwrap_or(u32::MAX),
                        u32::try_from(column.saturating_sub(1)).unwrap_or(u32::MAX),
                    )
                }),
            ),
            ConfigError::Io { path, .. } | ConfigError::TooLarge { path, .. } => {
                (path.clone(), None)
            }
        };
        // A path shown as `<external>/name` has no place under the root. Show the failure on
        // the project config then.
        let absolute = if path.starts_with('<') || path.is_empty() {
            self.root.join("slopvac.toml")
        } else {
            self.root.join(path)
        };
        LoadFailure {
            error: config_error(err, &self.root),
            path: absolute,
            position,
        }
    }

    /// The config that applies to the files in `dir`: the nearest one, else the config of
    /// the project root, so a buffer outside the project is linted as the project is.
    fn config_in(&self, dir: &Path) -> Result<Option<Arc<LoadedConfig>>, CliError> {
        match self.nearest_cached(dir)? {
            Some(found) => Ok(Some(found)),
            None => self.nearest_cached(&self.root),
        }
    }

    /// The nearest config of `dir`, read once between reloads.
    fn nearest_cached(&self, dir: &Path) -> Result<Option<Arc<LoadedConfig>>, CliError> {
        if let Some(found) = self.loaded().configs.get(dir) {
            return Ok(found.clone());
        }
        let found = nearest(dir).map_err(|e| config_error(&e, &self.root))?;
        self.loaded()
            .configs
            .insert(dir.to_path_buf(), found.clone());
        Ok(found)
    }

    /// Where the document at `path` sits: its canonical path, and the nearest directory
    /// above it that exists, where the config search starts. A buffer that was never saved
    /// has no file, and its folder may not exist either, but its path still selects the
    /// `[[overrides]]` and `[[genres]]` blocks. Outside any existing directory, the project
    /// root stands in.
    fn locate(&self, path: &Path) -> (PathBuf, PathBuf) {
        for ancestor in path.ancestors().skip(1) {
            if let Ok(real) = std::fs::canonicalize(ancestor)
                && let Ok(tail) = path.strip_prefix(ancestor)
            {
                return (real.join(tail), real);
            }
        }
        (path.to_path_buf(), self.root.clone())
    }

    /// The engine, relative path and genre of the document at `path`.
    fn resolve(&self, path: &str, text: &str) -> Result<Resolved, LintError> {
        if let Some(why) = self.loaded().failure.clone() {
            return Err(LintError(why));
        }
        self.resolve_settings(path, text)
            .map_err(|e| LintError(e.message))
    }

    fn resolve_settings(&self, path: &str, text: &str) -> Result<Resolved, CliError> {
        let file = Path::new(path);
        let (absolute, dir) = self.locate(file);
        let config = self.config_in(&dir)?;
        let none = LoadedConfig::without_files(self.root.clone());
        let (config_ref, rel) = match &config {
            Some(config) => (
                config.as_ref(),
                absolute
                    .strip_prefix(&config.root)
                    .map_or_else(|_| absolute.clone(), Path::to_path_buf),
            ),
            None => (&none, PathBuf::from(path)),
        };
        // The catalog stays in the shared state, so take what the settings need under the
        // lock and build the engine outside it.
        let (settings, terms, key) = {
            let loaded = self.loaded();
            let catalog = loaded
                .catalog
                .as_ref()
                .ok_or_else(|| CliError::new("rules-unavailable", "the rules are not loaded"))?;
            settings_for(catalog, config_ref, &rel, &Overrides::default())?
        };
        let cached = self.loaded().engines.get(&key).cloned();
        let engine = if let Some(engine) = cached {
            engine
        } else {
            // The engine owns its rule sets, and a set does not clone: load them again.
            let sets = Catalog::load()?.into_sets();
            let reasons = slopvac_lsp::exception_names(&sets);
            let built = lint_terms::build_engine(sets, settings, &terms, &self.root)?;
            let engine = Arc::new(EngineLinter::from_engine(built, reasons));
            Arc::clone(
                self.loaded()
                    .engines
                    .entry(key)
                    .or_insert_with(|| Arc::clone(&engine)),
            )
        };
        Ok(Resolved {
            engine,
            relative: config.as_ref().map(|_| rel_text(&rel)),
            genre: declared_genre_of(config.as_deref(), text, path),
        })
    }
}

impl Linter for ProjectLinter {
    fn lint(&self, path: &str, text: &str) -> Result<Linted, LintError> {
        let resolved = self.project.resolve(path, text)?;
        resolved.engine.lint_resolved(
            path,
            text,
            resolved.relative.as_deref(),
            resolved.genre.as_deref(),
        )
    }

    fn allow_reasons(&self, path: &str, text: &str, site: usize, rule: &RuleId) -> Vec<String> {
        self.project.resolve(path, text).map_or_else(
            |_| Vec::new(),
            |resolved| resolved.engine.allow_reasons(path, text, site, rule),
        )
    }

    fn reload(&self) -> Reloaded {
        match self.project.load() {
            Ok(notices) => Reloaded {
                notices,
                failure: None,
            },
            Err(failure) => {
                let mut shown = ConfigFailure::new(
                    failure.error.code,
                    failure.error.message,
                    failure.path.display().to_string(),
                );
                if let Some((line, column)) = failure.position {
                    shown = shown.at(line, column);
                }
                Reloaded {
                    notices: Vec::new(),
                    failure: Some(shown),
                }
            }
        }
    }

    fn watched(&self) -> Vec<String> {
        self.project.loaded().files.clone()
    }
}

impl Check for ProjectCheck {
    fn check(&self, path: &str, text: &str) -> Vec<Problem> {
        // A config that failed to load gives no policy to check against.
        if self.project.loaded().failure.is_some() {
            return Vec::new();
        }
        let (_, dir) = self.project.locate(Path::new(path));
        let Ok(config) = self.project.config_in(&dir) else {
            return Vec::new();
        };
        let policy = config
            .as_deref()
            .map_or_else(Policy::core, Policy::from_config);
        slopvac_lsp::FrontMatterCheck::new(policy).check(path, text)
    }
}
