//! The [`Linter`] over `slopvac-engine`.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use slopvac_engine::discover::FileKind;
use slopvac_engine::{Engine, Input, Settings};
use slopvac_model::{Notice, NoticeLevel};
use slopvac_rules::RuleSet;

use crate::capped::{Capped, Listed};
use crate::limits::{Budget, Kind, Limits};
use crate::messages::MigrateText;
use crate::walk::discover;
use crate::{Error, Finding, Linter, Phase};

/// The config file name that 3.0 reads.
const CONFIG_NAME: &str = "slopvac.toml";

/// The settings of one document under a loaded config, from the path of the document
/// relative to the config's directory and its text. The text carries the document's own
/// `slopvac:` block and inline directives, which a lint run reads too.
pub type SettingsOf<'a> = Box<dyn Fn(&Path, &str) -> Result<Settings, Error> + 'a>;

/// Turns a 3.0 config into engine settings, the way `slopvac lint` resolves it.
///
/// The CLI implements it over its own resolver, so migrate and lint share one bridge from a
/// config to the engine.
pub trait Configure {
    /// Load the config of `dir` and return the settings of each document under it.
    ///
    /// `config` is the text of `dir/slopvac.toml`. With `None`, the documents of `dir` take
    /// the core defaults.
    ///
    /// # Errors
    /// [`Error::Lint`] when the config does not load, names a category or style that is not
    /// shipped, or builds settings that fail validation.
    fn load<'a>(&'a self, dir: &Path, config: Option<&str>) -> Result<SettingsOf<'a>, Error>;
}

/// Lints the Markdown and MDX documents of a project with the engine.
///
/// Each document takes the settings of the nearest `slopvac.toml` at or below the project
/// root, through [`Configure`]. A config in the overlay replaces the one on disk, so the
/// before side reads what each 2.x config meant and the after side reads the migrated
/// configs. A document in the overlay replaces its text on disk the same way, so the before
/// side reads each 2.x directive under its 3.0 id. Files under
/// [`crate::BACKUP_DIR`], at any depth, are never linted. A document the engine cannot
/// read, or one it could not check in full, fails the lint.
pub struct EngineLinter {
    sets: Vec<Arc<RuleSet>>,
    configure: Box<dyn Configure>,
    limits: Limits,
}

impl EngineLinter {
    /// An engine over `sets`, with settings from `configure`.
    #[must_use]
    pub fn new(sets: Vec<RuleSet>, configure: Box<dyn Configure>) -> Self {
        Self {
            sets: sets.into_iter().map(Arc::new).collect(),
            configure,
            limits: Limits::default(),
        }
    }

    /// The same linter under other per-run limits.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
}

impl std::fmt::Debug for EngineLinter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineLinter").finish_non_exhaustive()
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// `path` with its directory resolved, for a file that may not exist yet.
fn canonical_file(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => canonical(dir).join(name),
        _ => canonical(path),
    }
}

/// The first warning-level notice, as a failure: part of the document was not checked.
fn unchecked(label: &str, notices: &[Notice]) -> Result<(), Error> {
    match notices.iter().find(|n| n.level == NoticeLevel::Warning) {
        Some(n) => Err(Error::Lint(
            MigrateText::LintNotFullyChecked {
                label: label.to_owned(),
                code: n.code.as_str().to_owned(),
                message: n.message.as_str().to_owned(),
            }
            .to_string(),
        )),
        None => Ok(()),
    }
}

/// One document to lint: its label, kind, text and resolved path, and its path relative to
/// the directory of its config when a config covers it.
struct Doc {
    label: String,
    kind: FileKind,
    text: String,
    path: PathBuf,
    rel: Option<String>,
}

impl Doc {
    /// The engine input. A document under a config carries its path relative to that
    /// config, which `[[genres]]` and override patterns match, as in `slopvac lint`.
    fn input(&self) -> Input<'_> {
        let input = match self.kind {
            FileKind::Mdx => Input::mdx(&self.label, &self.text),
            _ => Input::markdown(&self.label, &self.text),
        };
        match &self.rel {
            Some(rel) => input.with_relative_path(rel),
            None => input,
        }
    }
}

/// `rel` with `/` between its components, the form config patterns match.
fn slash_path(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

impl EngineLinter {
    /// The settings of each document, from the nearest config at or below `base`. Each
    /// document under a config gets its path relative to that config.
    fn settings_of(
        &self,
        base: &Path,
        docs: &mut [Doc],
        configs: &HashMap<PathBuf, String>,
    ) -> Result<Vec<Settings>, Error> {
        let mut loaded: HashMap<PathBuf, SettingsOf<'_>> = HashMap::new();
        let mut out = Vec::with_capacity(docs.len());
        for doc in docs.iter_mut() {
            let dir = doc
                .path
                .ancestors()
                .skip(1)
                .take_while(|dir| dir.starts_with(base))
                .find(|dir| configs.contains_key(&dir.join(CONFIG_NAME)))
                .unwrap_or(base)
                .to_owned();
            if !loaded.contains_key(&dir) {
                let config = configs.get(&dir.join(CONFIG_NAME)).map(String::as_str);
                let of = self.configure.load(&dir, config)?;
                loaded.insert(dir.clone(), of);
            }
            let rel = doc.path.strip_prefix(&dir).unwrap_or(&doc.path).to_owned();
            out.push(loaded[&dir](&rel, &doc.text)?);
            if configs.contains_key(&dir.join(CONFIG_NAME)) {
                doc.rel = Some(slash_path(&rel));
            }
        }
        Ok(out)
    }
}

impl Linter for EngineLinter {
    fn lint(
        &self,
        root: &Path,
        _phase: Phase,
        overlay: &[(PathBuf, String)],
    ) -> Result<Vec<Finding>, Error> {
        let base = canonical(root);
        let mut budget = Budget::new(self.limits);
        // The planner reports what discovery excludes; the lint only needs the documents.
        let found = discover(root, &mut budget, &mut Capped::new(Listed::Warnings))?;
        let replaced: HashMap<PathBuf, &str> = overlay
            .iter()
            .map(|(path, text)| (canonical_file(path), text.as_str()))
            .collect();
        let mut configs: HashMap<PathBuf, String> = HashMap::new();
        for (path, text) in &replaced {
            if path.file_name().is_some_and(|n| n == CONFIG_NAME) && path.starts_with(&base) {
                let shown = root.join(path.strip_prefix(&base).unwrap_or(path));
                budget.check_text(&shown, Kind::Config, text)?;
                configs.insert(path.clone(), (*text).to_owned());
            }
        }
        for dir in found.dirs.iter().filter(|d| d.main) {
            if let Entry::Vacant(slot) = configs.entry(canonical(&dir.path).join(CONFIG_NAME)) {
                let path = slot.key();
                let shown = root.join(path.strip_prefix(&base).unwrap_or(path));
                let text = budget.read(path, Kind::Config).map_err(|e| e.at(&shown))?;
                slot.insert(text);
            }
        }
        let mut docs = Vec::new();
        for (file, kind) in found.docs {
            let path = canonical(&file);
            let label = path.strip_prefix(&base).unwrap_or(&path).to_owned();
            let shown = root.join(&label);
            let text = match replaced.get(&path) {
                Some(text) => {
                    budget.check_text(&shown, Kind::Document, text)?;
                    (*text).to_owned()
                }
                None => budget
                    .read(&path, Kind::Document)
                    .map_err(|e| e.at(&shown))?,
            };
            docs.push(Doc {
                label: label.display().to_string(),
                kind,
                text,
                path,
                rel: None,
            });
        }
        let settings = self.settings_of(&base, &mut docs, &configs)?;
        // One engine per distinct settings, as `slopvac lint` groups its files.
        let mut groups: BTreeMap<String, (Settings, Vec<usize>)> = BTreeMap::new();
        for (index, s) in settings.into_iter().enumerate() {
            groups
                .entry(format!("{s:?}"))
                .or_insert_with(|| (s, Vec::new()))
                .1
                .push(index);
        }
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for (settings, indexes) in groups.into_values() {
            let engine = Engine::new_shared(self.sets.clone(), settings)
                .map_err(|e| Error::Lint(e.to_string()))?;
            let inputs: Vec<Input<'_>> = indexes.iter().map(|&i| docs[i].input()).collect();
            for (result, &i) in engine.lint_all(&inputs).into_iter().zip(&indexes) {
                let label = &docs[i].label;
                let doc = result.map_err(|e| {
                    Error::Lint(
                        MigrateText::LintDocumentFailed {
                            label: label.clone(),
                            detail: e.to_string(),
                        }
                        .to_string(),
                    )
                })?;
                unchecked(label, &doc.unchecked)?;
                for finding in doc.findings {
                    *counts.entry(finding.rule_id.to_string()).or_default() += 1;
                }
            }
        }
        Ok(counts
            .into_iter()
            .map(|(rule, count)| Finding { rule, count })
            .collect())
    }
}
