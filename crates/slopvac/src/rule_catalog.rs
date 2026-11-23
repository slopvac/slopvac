//! The rules a run knows: the core pack, the synced plugins that the config lists, and the
//! term-casing rules.
//!
//! The core pack and the synced plugins load through `slopvac-packs`, so each pack's budgets
//! (docs/contract/policies.md, section 1.9) and its registry checks apply to every run.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value as Json;
use slopvac_config::LoadedConfig;
use slopvac_model::{PackId, RuleId};
use slopvac_packs::{PackError, Packs, SyncedError};
use slopvac_plugins::lock::{LockError, Lockfile};
use slopvac_plugins::sync::Store;
use slopvac_rules::{LoadedCategory, LoadedRule, RuleSet};

use crate::doctor::read::{ReadProblem, read_bounded};
use crate::errors::CliError;

/// The error code of a refusal that has no code of its own.
const FALLBACK_CODE: &str = "rules-unavailable";

/// The error for a pack that did not load. The code is the pack loader's own, so a budget
/// refusal is `rules-budget-exceeded` and a plugin the lock lacks is `plugin-not-locked`.
fn pack_refusal(code: Option<&'static str>, text: &str) -> CliError {
    CliError::escaping(code.unwrap_or(FALLBACK_CODE), text)
}

fn core_refusal(err: &PackError) -> CliError {
    pack_refusal(err.code(), &err.to_string())
}

fn synced_refusal(err: &SyncedError) -> CliError {
    pack_refusal(err.code(), &err.to_string())
}

/// Load the core rules.
///
/// # Errors
/// The code of the pack loader's refusal, or `rules-unavailable` when it has none.
pub fn core() -> Result<RuleSet, CliError> {
    let mut sets = Packs::core()
        .map_err(|e| core_refusal(&e))?
        .into_rule_sets();
    // `Packs::core` loads exactly one pack.
    sets.pop()
        .ok_or_else(|| CliError::new(FALLBACK_CODE, "the core pack has no rules"))
}

/// Which synced plugins a run loads besides the core pack.
///
/// The config decides which plugins load (threat model X6). The lock pins each one and the
/// store holds its tree.
#[derive(Debug, Clone, Default)]
pub struct PluginSelection {
    requested: Vec<PackId>,
    lock: Lockfile,
    store_root: PathBuf,
}

impl PluginSelection {
    /// The plugins that `config` lists under `[[plugins]]`, with the lock of `root` and the
    /// per-user store. A config without plugins reads neither.
    ///
    /// # Errors
    /// `rules-unavailable` when the lock or the store location cannot be read.
    pub fn from_config(
        root: &std::path::Path,
        config: Option<&LoadedConfig>,
    ) -> Result<Self, CliError> {
        let requested: Vec<PackId> = config
            .and_then(|c| c.project_value(&["plugins"]))
            .iter()
            .filter_map(Json::as_array)
            .flatten()
            .filter_map(|entry| entry.get("id")?.as_str())
            .filter_map(|id| PackId::new(id).ok())
            .collect();
        if requested.is_empty() {
            return Ok(Self::default());
        }
        let lock_path = root.join(".slopvac").join("plugins.lock");
        let lock = match read_bounded(&lock_path, slopvac_plugins::lock::MAX_LOCK_BYTES as u64) {
            Ok(None) => Lockfile::default(),
            Ok(Some(text)) => Lockfile::parse(&text).map_err(|e| lock_refusal(&e))?,
            Err(problem) => {
                let why = match problem {
                    ReadProblem::NotRegular => "is not a regular file",
                    ReadProblem::TooLarge => "is larger than the lockfile limit",
                    ReadProblem::Io => "cannot be read",
                };
                return Err(CliError::new(
                    FALLBACK_CODE,
                    format!(".slopvac/plugins.lock {why}"),
                ));
            }
        };
        let store_root = slopvac_fs::data_dir().map_err(|e| {
            CliError::new(
                FALLBACK_CODE,
                format!("plugin store: no data directory: {e}"),
            )
        })?;
        Ok(Self {
            requested,
            lock,
            store_root,
        })
    }

    /// Load the core pack, the selected plugins and the term-casing rules.
    ///
    /// # Errors
    /// The code of the pack loader's refusal, or `rules-unavailable` when it has none, and
    /// `engine-unavailable` when the term-casing pack does not load.
    pub fn load(&self) -> Result<Catalog, CliError> {
        if self.requested.is_empty() {
            return Catalog::load();
        }
        let mut sets =
            Packs::with_synced(&self.requested, &self.lock, &Store::new(&self.store_root))
                .map_err(|e| synced_refusal(&e))?
                .into_rule_sets();
        sets.push(crate::lint_terms::casing_rule_set()?);
        Ok(Catalog {
            sets,
            selection: Arc::new(self.clone()),
        })
    }
}

fn lock_refusal(err: &LockError) -> CliError {
    CliError::escaping(FALLBACK_CODE, &format!(".slopvac/plugins.lock: {err}"))
}

/// Every rule set a run knows: the core rules, the plugins' and the term-casing rules. Config
/// validation, the effective-severity commands and the engine all read the same catalogue.
#[derive(Debug)]
pub struct Catalog {
    sets: Vec<RuleSet>,
    selection: Arc<PluginSelection>,
}

impl Catalog {
    /// Load the core and term-casing rules.
    ///
    /// # Errors
    /// The code of the pack loader's refusal, or `rules-unavailable` when it has none, and
    /// `engine-unavailable` when the term-casing pack does not load.
    pub fn load() -> Result<Self, CliError> {
        Ok(Self {
            sets: vec![core()?, crate::lint_terms::casing_rule_set()?],
            selection: Arc::default(),
        })
    }

    /// A catalogue over `sets`, the first being the core rules.
    #[must_use]
    pub fn from_sets(sets: Vec<RuleSet>) -> Self {
        Self {
            sets,
            selection: Arc::default(),
        }
    }

    /// The plugins this catalogue was loaded with, to load the same rules again.
    #[must_use]
    pub fn selection(&self) -> Arc<PluginSelection> {
        Arc::clone(&self.selection)
    }

    /// The rule sets, core first.
    #[must_use]
    pub fn sets(&self) -> &[RuleSet] {
        &self.sets
    }

    /// The rule sets, to hand to the engine.
    #[must_use]
    pub fn into_sets(self) -> Vec<RuleSet> {
        self.sets
    }

    /// Every category, core first.
    pub fn categories(&self) -> impl Iterator<Item = &LoadedCategory> {
        self.sets.iter().flat_map(|s| s.categories().iter())
    }

    /// Every rule, core first.
    pub fn rules(&self) -> impl Iterator<Item = &LoadedRule> {
        self.sets.iter().flat_map(RuleSet::rules)
    }

    /// The rule with this id.
    #[must_use]
    pub fn get(&self, id: &RuleId) -> Option<&LoadedRule> {
        self.sets.iter().find_map(|s| s.get(id))
    }

    /// The category a config or command-line key names, bare or qualified. A qualified key
    /// names a category of that pack only: `terms:orwell` is not core's `orwell`.
    #[must_use]
    pub fn category_named(&self, key: &str) -> Option<&LoadedCategory> {
        match key.split_once(':') {
            Some((pack, name)) => self
                .sets
                .iter()
                .find(|s| s.pack().as_str() == pack)?
                .categories()
                .iter()
                .find(|c| c.id().as_str() == name),
            None => self.categories().find(|c| c.id().as_str() == key),
        }
    }

    /// The rule a config or command-line key names, bare or qualified. A bare key reads as
    /// the first pack that has such a rule.
    #[must_use]
    pub fn rule_named(&self, key: &str) -> Option<&LoadedRule> {
        if key.contains(':') {
            return self.rules().find(|r| r.id().as_str() == key);
        }
        self.rules().find(|r| r.id().local() == key)
    }
}
