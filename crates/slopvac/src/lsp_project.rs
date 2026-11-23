//! The language server's view of the project: the settings of each open document, and the
//! reload when a config file changes.
//!
//! A document gets the settings `slopvac lint` would give it: the nearest `slopvac.toml`
//! of its directory, with the `[[overrides]]` and `[[genres]]` blocks that match its path
//! (see [`crate::engine::settings_for`]). A buffer with no config on its path gets the core
//! defaults, as a file does in a lint run. One engine serves every document whose settings
//! are equal.
//!
//! A reload throws away every loaded config and engine, reads the project config again and
//! reports what it found. A config that cannot be read leaves no document looking checked:
//! every document is reported as not linted, with the config error as the reason, until a
//! later reload succeeds. The earlier settings are not kept, because a result computed from
//! them would pass for a result of the config on disk.
//!
//! Every load counts an epoch. Work that began under an earlier epoch (a config read, an
//! engine build) may finish after a reload, and its result must not enter the caches of the
//! new one, so each insert names the epoch it began under and is dropped when that is no
//! longer current.
//!
//! The caches hold at most [`MAX_CONFIGS`] directories and [`MAX_ENGINES`] engines. A full
//! cache is emptied before the next entry goes in. Every entry can be built again from disk,
//! and a lint that holds an engine keeps it alive through its `Arc`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use slopvac_config::frontmatter::Policy;
use slopvac_config::{ConfigError, Discovery, LoadedConfig};
use slopvac_lsp::{
    Check, ConfigFailure, EngineLinter, LintError, Linted, Linter, Problem, Reloaded,
};
use slopvac_model::{ConfigSource, Notice, RuleId};
use slopvac_styles::{BuiltinSource, StylePresets};

use crate::engine::{config_error, declared_genre, read_block_under, rel_text, settings_for};
use crate::errors::CliError;
use crate::lint_settings::{self, Located, Overrides};
use crate::lint_terms;
use crate::rule_catalog::Catalog;

/// How many directories the config cache holds.
const MAX_CONFIGS: usize = 256;
/// How many engines the engine cache holds. An engine owns its compiled rule sets, so this
/// is the smaller bound.
const MAX_ENGINES: usize = 16;

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
    /// The text of that file and the byte offset of the error in it.
    position: Option<(String, usize)>,
}

/// A map that holds at most `cap` entries: a new key that finds it full empties it first.
struct Bounded<K, V> {
    map: HashMap<K, V>,
    cap: usize,
}

impl<K: Eq + Hash, V> Bounded<K, V> {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            cap,
        }
    }

    fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.get(key)
    }

    fn insert(&mut self, key: K, value: V) {
        if self.map.len() >= self.cap && !self.map.contains_key(&key) {
            self.map.clear();
        }
        self.map.insert(key, value);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }
}

/// What a successful load of the project holds.
struct Ready {
    catalog: Catalog,
    /// The nearest config of each directory, as `slopvac lint` finds it.
    configs: Bounded<PathBuf, Option<Arc<LoadedConfig>>>,
    /// One engine for each distinct settings set.
    engines: Bounded<String, Arc<EngineLinter>>,
    /// The warnings shown so far, so a config that several directories share warns once.
    notified: HashSet<(String, String)>,
    /// Warnings from configs that linting found, not yet handed to the server.
    pending: Vec<Notice>,
}

/// The result of the last load. A reload replaces all of it.
enum State {
    /// The project config could not be loaded. The text says why.
    Failed(String),
    Ready(Ready),
}

struct Loaded {
    /// Counts loads. Work that began under another value is stale.
    epoch: u64,
    /// Every config file the active configs read or tried to read, absolute and normalised:
    /// the files to watch.
    dependencies: BTreeSet<PathBuf>,
    state: State,
}

/// A point in a lookup that a test can pause at, to interleave a reload there.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// A config was read from disk and is not in the cache yet.
    ConfigRead,
    /// An engine was built and is not in the cache yet.
    EngineBuilt,
}

#[cfg(test)]
type Pause = Arc<dyn Fn(Stage) + Send + Sync>;

struct Project {
    root: PathBuf,
    loaded: Mutex<Loaded>,
    #[cfg(test)]
    pause: Mutex<Option<Pause>>,
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
        loaded: Mutex::new(Loaded {
            epoch: 0,
            dependencies: BTreeSet::new(),
            // Replaced by the load below. A project whose first load fails is never served.
            state: State::Failed(String::new()),
        }),
        #[cfg(test)]
        pause: Mutex::new(None),
    });
    let notices = project.load(&[]).map_err(|failure| failure.error)?;
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

/// The text of the config file at `path` and the byte offset of `source` in it. `None` when
/// the source has no line, or the file cannot be read again.
fn source_position(path: &Path, source: &ConfigSource) -> Option<(String, usize)> {
    let (line, column) = (source.line?.get(), source.column?.get());
    let text = slopvac_fs::read_capped_regular_to_string(
        path,
        slopvac_config::MAX_CONFIG_BYTES,
        slopvac_fs::ReadOptions::new(),
    )
    .ok()?;
    let start: usize = text
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum();
    let rest = text.get(start..)?;
    let offset = start
        + rest
            .char_indices()
            .nth(column.saturating_sub(1))
            .map_or(rest.len(), |(at, _)| at);
    Some((text, offset))
}

/// Add `files` to the set of files to watch, each under the name the config gave it and, when
/// that is a link, under the name of the file it points to as well: an editor reports a change
/// under either. A file that does not exist yet has no other name.
fn watch_all(watched: &mut BTreeSet<PathBuf>, files: &[PathBuf]) {
    for file in files {
        watched.insert(file.clone());
        if let Ok(real) = std::fs::canonicalize(file) {
            watched.insert(real);
        }
    }
}

fn notice_key(notice: &Notice) -> (String, String) {
    (notice.code.to_string(), notice.message.as_str().to_owned())
}

impl Project {
    fn loaded(&self) -> MutexGuard<'_, Loaded> {
        self.loaded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The state, when no load has replaced it since `epoch`.
    fn current(&self, epoch: u64) -> Option<MutexGuard<'_, Loaded>> {
        let loaded = self.loaded();
        (loaded.epoch == epoch).then_some(loaded)
    }

    #[cfg(test)]
    fn paused_at(&self, stage: Stage) {
        let hook = self
            .pause
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook(stage);
        }
    }

    /// Replace everything with a fresh read of the project config. `open` holds the paths of
    /// the documents open now: the config of each one's folder is checked as well, so a break
    /// in one of them fails the reload. The folders come from the open documents each time, so
    /// a failed reload or a full cache forgets none, and a closed document leaves none behind.
    fn load(&self, open: &[String]) -> Result<Vec<Notice>, Box<LoadFailure>> {
        let mut dependencies = BTreeSet::new();
        let active: BTreeSet<PathBuf> = open
            .iter()
            .map(|path| self.locate(Path::new(path)).1)
            .collect();
        let result = self.read(&mut dependencies, &active);
        let mut loaded = self.loaded();
        let epoch = loaded.epoch + 1;
        match result {
            Ok((ready, notices)) => {
                *loaded = Loaded {
                    epoch,
                    dependencies,
                    state: State::Ready(ready),
                };
                Ok(notices)
            }
            Err(failure) => {
                *loaded = Loaded {
                    epoch,
                    dependencies,
                    state: State::Failed(failure.error.message.clone()),
                };
                Err(failure)
            }
        }
    }

    fn read(
        &self,
        dependencies: &mut BTreeSet<PathBuf>,
        active: &BTreeSet<PathBuf>,
    ) -> Result<(Ready, Vec<Notice>), Box<LoadFailure>> {
        let fallback = |error: CliError| {
            Box::new(LoadFailure {
                error,
                path: self.root.join("slopvac.toml"),
                position: None,
            })
        };
        let catalog = Catalog::load().map_err(fallback)?;
        let config = nearest(&self.root).map_err(|e| {
            watch_all(dependencies, e.dependencies());
            Box::new(self.failure_of(&e))
        })?;
        if let Some(config) = &config {
            watch_all(dependencies, config.dependencies());
        }
        let notices = match &config {
            Some(config) => lint_settings::validate_located(config, &catalog)
                .map_err(|located| Box::new(self.located_failure(config, *located)))?,
            None => Vec::new(),
        };
        let mut ready = Ready {
            catalog,
            configs: Bounded::new(MAX_CONFIGS),
            engines: Bounded::new(MAX_ENGINES),
            notified: notices.iter().map(notice_key).collect(),
            pending: Vec::new(),
        };
        ready.configs.insert(self.root.clone(), config);
        let mut notices = notices;
        let presets = StylePresets(BuiltinSource);
        let discovery = Discovery::new(&presets);
        for dir in active.iter().filter(|dir| **dir != self.root) {
            let found = match discovery.nearest(dir) {
                Ok(found) => found,
                // The folder is gone, and so is whatever was open in it.
                Err(ConfigError::Io { file: None, .. }) => continue,
                Err(error) => {
                    watch_all(dependencies, error.dependencies());
                    return Err(Box::new(self.failure_of(&error)));
                }
            };
            if let Some(config) = &found {
                watch_all(dependencies, config.dependencies());
                let fresh = lint_settings::validate_located(config, &ready.catalog)
                    .map_err(|located| Box::new(self.located_failure(config, *located)))?;
                for notice in fresh {
                    if ready.notified.insert(notice_key(&notice)) {
                        notices.push(notice);
                    }
                }
            }
            ready.configs.insert(dir.clone(), found);
        }
        Ok((ready, notices))
    }

    /// Where a validation failure shows: on the file its key is in, at the key.
    fn located_failure(&self, config: &LoadedConfig, located: Located) -> LoadFailure {
        let own = config
            .dependencies()
            .first()
            .cloned()
            .unwrap_or_else(|| self.root.join("slopvac.toml"));
        let (path, position) = match &located.source {
            Some(source) => {
                let path = source.file.as_ref().map_or(own, |file| {
                    slopvac_fs::lexical_normalize(&config.root.join(file))
                });
                let position = source_position(&path, source);
                (path, position)
            }
            None => (own, None),
        };
        LoadFailure {
            error: located.error,
            path,
            position,
        }
    }

    /// Where a config error shows: on the file it is about, which sits outside the project
    /// when an `extends` entry leads there, at the position the error names.
    fn failure_of(&self, err: &ConfigError) -> LoadFailure {
        let position = match err {
            ConfigError::Rejected(rejected) => rejected
                .diagnostics
                .first()
                .map(|d| (rejected.text.clone(), d.span.start)),
            ConfigError::Io { .. } | ConfigError::TooLarge { .. } => None,
        };
        LoadFailure {
            error: config_error(err, &self.root),
            path: err
                .source_file()
                .map_or_else(|| self.root.join("slopvac.toml"), Path::to_path_buf),
            position,
        }
    }

    /// The config that applies to the files in `dir`: the nearest one, else none, so a buffer
    /// with no config on its path gets the core defaults, as a file does in a lint run.
    fn config_in(&self, dir: &Path, epoch: u64) -> Result<Option<Arc<LoadedConfig>>, CliError> {
        if let State::Ready(ready) = &self.loaded().state
            && let Some(found) = ready.configs.get(dir)
        {
            return Ok(found.clone());
        }
        let found = nearest(dir);
        #[cfg(test)]
        self.paused_at(Stage::ConfigRead);
        let Some(mut loaded) = self.current(epoch) else {
            // A reload ran meanwhile. The answer is for the old config: use it, keep nothing.
            return found.map_err(|e| config_error(&e, &self.root));
        };
        let loaded = &mut *loaded;
        match found {
            Ok(config) => {
                if let Some(config) = &config {
                    watch_all(&mut loaded.dependencies, config.dependencies());
                }
                let State::Ready(ready) = &mut loaded.state else {
                    return Ok(config);
                };
                // The same checks `slopvac lint` runs on each config it loads.
                if let Some(config) = &config {
                    for notice in lint_settings::validate(config, &ready.catalog)? {
                        if ready.notified.insert(notice_key(&notice)) {
                            ready.pending.push(notice);
                        }
                    }
                }
                ready.configs.insert(dir.to_path_buf(), config.clone());
                Ok(config)
            }
            Err(error) => {
                watch_all(&mut loaded.dependencies, error.dependencies());
                Err(config_error(&error, &self.root))
            }
        }
    }

    /// Where the document at `path` sits: its canonical path, link resolved, and the directory
    /// the config search starts in. A buffer that was never saved has no file, and its folder
    /// may not exist either, but its path still selects the `[[overrides]]` and `[[genres]]`
    /// blocks: the search then starts in the nearest directory above it that exists. Outside
    /// any existing directory, the project root stands in.
    fn locate(&self, path: &Path) -> (PathBuf, PathBuf) {
        // `slopvac lint` resolves a file that exists before it looks for its config, so a
        // link takes the settings of the file it points to.
        if let Ok(real) = std::fs::canonicalize(path)
            && let Some(dir) = real.parent()
        {
            let dir = dir.to_path_buf();
            return (real, dir);
        }
        for ancestor in path.ancestors().skip(1) {
            if let Ok(real) = std::fs::canonicalize(ancestor)
                && let Ok(tail) = path.strip_prefix(ancestor)
            {
                return (real.join(tail), real);
            }
        }
        (path.to_path_buf(), self.root.clone())
    }

    /// The epoch of the loaded project, or why the project cannot lint.
    fn epoch(&self) -> Result<u64, LintError> {
        let loaded = self.loaded();
        match &loaded.state {
            State::Failed(why) => Err(LintError(why.clone())),
            State::Ready(_) => Ok(loaded.epoch),
        }
    }

    /// The engine, relative path and genre of the document at `path`.
    fn resolve(&self, path: &str, text: &str) -> Result<Resolved, LintError> {
        let epoch = self.epoch()?;
        self.resolve_settings(path, text, epoch)
    }

    fn resolve_settings(&self, path: &str, text: &str, epoch: u64) -> Result<Resolved, LintError> {
        let shown = |e: CliError| LintError(e.message);
        let file = Path::new(path);
        let (absolute, dir) = self.locate(file);
        let config = self.config_in(&dir, epoch).map_err(shown)?;
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
        let (settings, terms, key, cached) = {
            let loaded = self.loaded();
            let ready = match &loaded.state {
                State::Ready(ready) => ready,
                State::Failed(why) => return Err(LintError(why.clone())),
            };
            let (settings, terms, key) =
                settings_for(&ready.catalog, config_ref, &rel, &Overrides::default())
                    .map_err(shown)?;
            let cached = ready.engines.get(&key).cloned();
            (settings, terms, key, cached)
        };
        let engine = if let Some(engine) = cached {
            engine
        } else {
            // The engine owns its rule sets, and a set does not clone: load them again.
            let sets = Catalog::load().map_err(shown)?.shared_sets();
            let reasons = slopvac_lsp::exception_names(&sets);
            let built =
                lint_terms::build_engine(sets, settings, &terms, &self.root).map_err(shown)?;
            let engine = Arc::new(EngineLinter::from_engine(built, reasons));
            #[cfg(test)]
            self.paused_at(Stage::EngineBuilt);
            match self.current(epoch) {
                Some(mut loaded) => match &mut loaded.state {
                    State::Ready(ready) => {
                        if let Some(existing) = ready.engines.get(&key) {
                            Arc::clone(existing)
                        } else {
                            ready.engines.insert(key, Arc::clone(&engine));
                            engine
                        }
                    }
                    State::Failed(_) => engine,
                },
                None => engine,
            }
        };
        Ok(Resolved {
            engine,
            relative: config.as_ref().map(|_| rel_text(&rel)),
            genre: declared_genre(&read_block_under(text, path, config.as_deref())),
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

    fn reload(&self, open: &[String]) -> Reloaded {
        match self.project.load(open) {
            Ok(notices) => Reloaded::new(notices, None),
            Err(failure) => {
                let mut shown =
                    ConfigFailure::new(failure.error.code, failure.error.message, failure.path);
                if let Some((text, offset)) = failure.position {
                    shown = shown.at_offset(text, offset);
                }
                Reloaded::new(Vec::new(), Some(shown))
            }
        }
    }

    fn watched(&self) -> Vec<String> {
        self.project
            .loaded()
            .dependencies
            .iter()
            .map(|p| p.display().to_string())
            .collect()
    }

    fn watches(&self, path: &str) -> bool {
        // `a/../b` and `b` are one file; the dependencies are stored in the second form.
        let path = slopvac_fs::lexical_normalize(Path::new(path));
        self.project.loaded().dependencies.contains(&path)
    }

    fn take_notices(&self) -> Vec<Notice> {
        match &mut self.project.loaded().state {
            State::Ready(ready) => std::mem::take(&mut ready.pending),
            State::Failed(_) => Vec::new(),
        }
    }
}

impl Check for ProjectCheck {
    fn check(&self, path: &str, text: &str) -> Vec<Problem> {
        // A config that failed to load gives no policy to check against.
        let Ok(epoch) = self.project.epoch() else {
            return Vec::new();
        };
        let (_, dir) = self.project.locate(Path::new(path));
        let Ok(config) = self.project.config_in(&dir, epoch) else {
            return Vec::new();
        };
        let policy = config
            .as_deref()
            .map_or_else(Policy::core, Policy::from_config);
        slopvac_lsp::FrontMatterCheck::new(policy).check(path, text)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::Duration;

    use super::*;

    const WAIT: Duration = Duration::from_secs(20);
    const SLOPPY: &str = "We leverage a robust, seamless solution to delve into the data.\n";
    const LEXICON: &str = "core:prose-inflation.slop-lexicon";
    const LEXICON_OFF: &str =
        "schema_version = 3\n\n[rules]\n\"prose-inflation.slop-lexicon\" = \"off\"\n";

    #[test]
    fn a_full_cache_is_emptied_before_a_new_key_goes_in() {
        let mut cache = Bounded::new(2);
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get("c"), Some(&3));
        assert_eq!(cache.get("a"), None);
    }

    #[test]
    fn replacing_a_key_of_a_full_cache_keeps_the_other_entries() {
        let mut cache = Bounded::new(2);
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("b", 3);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get("a"), Some(&1));
        assert_eq!(cache.get("b"), Some(&3));
    }

    /// A project with a nested config in `sub/` that turns the lexicon rule off.
    fn nested_project() -> (tempfile::TempDir, PathBuf, ProjectServer) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().canonicalize().expect("canonical");
        std::fs::create_dir(root.join("sub")).expect("mkdir");
        std::fs::write(root.join("slopvac.toml"), "schema_version = 3\n").expect("config");
        std::fs::write(root.join("sub/slopvac.toml"), LEXICON_OFF).expect("nested config");
        let server = project_linter(&root).expect("a project");
        (dir, root, server)
    }

    /// Make the next lookup that reaches `stage` report on the first channel and wait for the
    /// second. Other stages pass.
    fn pause_at(project: &Project, stage: Stage) -> (Receiver<()>, Sender<()>) {
        let (reached_tx, reached) = channel();
        let (proceed, proceed_rx) = channel::<()>();
        let proceed_rx = Mutex::new(proceed_rx);
        let reached_tx = Mutex::new(reached_tx);
        *project.pause.lock().unwrap() = Some(Arc::new(move |at| {
            if at == stage {
                reached_tx.lock().unwrap().send(()).unwrap();
                proceed_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
            }
        }));
        (reached, proceed)
    }

    fn lexicon_found(linter: &ProjectLinter, path: &Path) -> bool {
        let linted = linter
            .lint(&path.display().to_string(), SLOPPY)
            .expect("linted");
        linted
            .findings
            .iter()
            .any(|f| f.rule_id.to_string() == LEXICON)
    }

    #[test]
    fn a_config_read_that_a_reload_overtook_does_not_refill_the_cache() {
        let (_dir, root, server) = nested_project();
        let linter = server.linter;
        let project = Arc::clone(&linter.project);
        let (reached, proceed) = pause_at(&project, Stage::ConfigRead);
        let document = root.join("sub/a.md");
        std::thread::scope(|scope| {
            let early = scope.spawn(|| lexicon_found(&linter, &document));
            // The lookup has read the nested config and is about to cache it.
            reached.recv_timeout(WAIT).unwrap();
            std::fs::write(root.join("sub/slopvac.toml"), "schema_version = 3\n").unwrap();
            linter.reload(&[document.display().to_string()]);
            proceed.send(()).unwrap();
            // It read the config before the edit, so what it saw is out of date.
            assert!(!early.join().unwrap());
        });
        *project.pause.lock().unwrap() = None;
        assert!(lexicon_found(&linter, &document));
    }

    #[test]
    fn an_engine_built_before_a_reload_is_not_cached_after_it() {
        let (_dir, root, server) = nested_project();
        let linter = server.linter;
        let project = Arc::clone(&linter.project);
        let (reached, proceed) = pause_at(&project, Stage::EngineBuilt);
        let document = root.join("a.md");
        std::thread::scope(|scope| {
            let early = scope.spawn(|| lexicon_found(&linter, &document));
            reached.recv_timeout(WAIT).unwrap();
            linter.reload(&[]);
            proceed.send(()).unwrap();
            early.join().unwrap();
        });
        let State::Ready(ready) = &project.loaded().state else {
            panic!("the reload succeeded");
        };
        assert_eq!(ready.engines.len(), 0);
    }
}
