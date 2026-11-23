//! The rules a run knows: the core pack, the synced plugins that the config lists, and the
//! term-casing rules.
//!
//! The core pack and the synced plugins load through `slopvac-packs`, so each pack's budgets
//! (docs/contract/policies.md, section 1.9) and its registry checks apply to every run.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value as Json;
use slopvac_config::{Discovery, LoadedConfig};
use slopvac_fs::{InputKind, ReadOptions, read_capped_regular_to_string};
use slopvac_model::{PackId, RuleId};
use slopvac_packs::{PackError, Packs, SyncedError};
use slopvac_plugins::lock::{LockError, Lockfile};
use slopvac_plugins::sync::Store;
use slopvac_rules::{LoadedCategory, LoadedRule, PackBudget, RuleSet};
use slopvac_styles::{BuiltinSource, StylePresets};

use crate::errors::CliError;
use crate::lint_messages::RuleSourceError;

/// The error for a pack that did not load. The code is the pack loader's own, so a budget
/// refusal is `rules-budget-exceeded` with its `budget` and `limit`, and a plugin the lock
/// lacks is `plugin-not-locked`.
fn pack_refusal(code: Option<&'static str>, text: &str, budget: Option<(PackBudget, u64)>) -> CliError {
    let error = match code {
        Some(code) => CliError::escaping(code, text),
        None => RuleSourceError::Unavailable {
            detail: slopvac_sanitize::line(text),
        }
        .into(),
    };
    match budget {
        Some((budget, limit)) => {
            let mut data = serde_json::Map::new();
            data.insert("budget".into(), budget.as_str().into());
            data.insert("limit".into(), limit.into());
            error.with_data(data)
        }
        None => error,
    }
}

fn core_refusal(err: &PackError) -> CliError {
    pack_refusal(err.code(), &err.to_string(), err.budget())
}

fn synced_refusal(err: &SyncedError) -> CliError {
    let budget = match err {
        SyncedError::Pack(pack) => pack.budget(),
        _ => None,
    };
    pack_refusal(err.code(), &err.to_string(), budget)
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
    sets.pop().ok_or_else(|| {
        RuleSourceError::Unavailable {
            detail: "the core pack has no rules".into(),
        }
        .into()
    })
}

/// The ids of the plugins that `config` lists under `[[plugins]]`.
fn requested_plugins(config: Option<&LoadedConfig>) -> Vec<PackId> {
    config
        .and_then(|c| c.project_value(&["plugins"]))
        .iter()
        .filter_map(Json::as_array)
        .flatten()
        .filter_map(|entry| entry.get("id")?.as_str())
        .filter_map(|id| PackId::new(id).ok())
        .collect()
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
    /// The plugins that `config` lists under `[[plugins]]`, with the lock that sits beside
    /// that config (`.slopvac/plugins.lock` in the config's own directory) and the per-user
    /// store. A config without plugins reads neither. `fallback_root` stands in for the
    /// config's directory when the run has no config.
    ///
    /// # Errors
    /// `run-plugins-lock-unreadable`, `run-plugins-lock-invalid` or `plugins-store-unavailable` when
    /// the lock or the store location cannot be read.
    pub fn from_config(
        config: Option<&LoadedConfig>,
        fallback_root: &Path,
    ) -> Result<Self, CliError> {
        let requested = requested_plugins(config);
        if requested.is_empty() {
            return Ok(Self::default());
        }
        let root = config.map_or(fallback_root, |c| c.root.as_path());
        let lock_path = root.join(".slopvac").join("plugins.lock");
        let cap = slopvac_plugins::lock::MAX_LOCK_BYTES as u64;
        let lock = match read_capped_regular_to_string(&lock_path, cap, ReadOptions::new()) {
            Ok(text) => Lockfile::parse(&text).map_err(|e| lock_refusal(&e))?,
            Err(err) => match err.input_kind() {
                Some(InputKind::NotFound) => Lockfile::default(),
                kind => {
                    let why = match kind {
                        Some(InputKind::NotRegular) => "is not a regular file",
                        Some(InputKind::Symlink) => "is a link",
                        Some(InputKind::TooLarge) => "is larger than the lockfile limit",
                        Some(InputKind::NotUtf8) => "is not valid UTF-8",
                        _ => "cannot be read",
                    };
                    return Err(RuleSourceError::LockUnreadable { why: why.into() }.into());
                }
            },
        };
        let store_root = slopvac_fs::data_dir().map_err(|_| RuleSourceError::StoreUnavailable)?;
        Ok(Self {
            requested,
            lock,
            store_root,
        })
    }

    /// Whether every plugin `config` lists is one this selection loads.
    #[must_use]
    pub fn covers(&self, config: &LoadedConfig) -> bool {
        requested_plugins(Some(config))
            .iter()
            .all(|id| self.requested.contains(id))
    }

    /// Load the core pack, the selected plugins and the term-casing rules, once. The packs
    /// compile under one budget, and every engine of the run shares the result.
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
        refuse_shared_categories(&sets)?;
        Ok(Catalog::from_sets(sets))
    }
}

/// Refuse two packs that supply a category with the same id. The engine keys category
/// settings and scoring by the category id alone, so a plugin's `orwell` would overwrite the
/// core's and `acme:orwell = off` would switch the core's category off.
fn refuse_shared_categories(sets: &[RuleSet]) -> Result<(), CliError> {
    let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for set in sets {
        for category in set.categories() {
            if let Some(first) = seen.insert(category.id().as_str(), set.pack().as_str()) {
                return Err(RuleSourceError::CategoryShared {
                    first: first.to_owned(),
                    second: set.pack().as_str().to_owned(),
                    category: category.id().as_str().to_owned(),
                }
                .into());
            }
        }
    }
    Ok(())
}

/// The lockfile's refusal names where the fault is and never what the file says there.
fn lock_refusal(err: &LockError) -> CliError {
    let at = if err.pointer.is_empty() {
        "/".to_owned()
    } else {
        slopvac_sanitize::line(&err.pointer)
    };
    RuleSourceError::LockInvalid { at }.into()
}

/// Every rule set a run knows: the core rules, the plugins' and the term-casing rules. Config
/// validation, the effective-severity commands and the engine all read the same catalogue.
#[derive(Debug, Clone)]
pub struct Catalog {
    sets: Vec<Arc<RuleSet>>,
}

impl Catalog {
    /// Load the core and term-casing rules.
    ///
    /// # Errors
    /// The code of the pack loader's refusal, or `rules-unavailable` when it has none, and
    /// `engine-unavailable` when the term-casing pack does not load.
    pub fn load() -> Result<Self, CliError> {
        Ok(Self::from_sets(vec![
            core()?,
            crate::lint_terms::casing_rule_set()?,
        ]))
    }

    /// The catalogue of the project at `root`: the core rules and the plugins that the nearest
    /// config lists. `slopvac lint` loads the same rules, so `rules` and `explain` name every
    /// rule that a lint can report. A config that cannot be read counts as no config here;
    /// `slopvac lint` reports it.
    ///
    /// # Errors
    /// As [`PluginSelection::load`] and [`PluginSelection::from_config`].
    pub fn configured(root: &Path) -> Result<Self, CliError> {
        let presets = StylePresets(BuiltinSource);
        let nearest = Discovery::new(&presets).nearest(root).ok().flatten();
        PluginSelection::from_config(nearest.as_deref(), root)?.load()
    }

    /// A catalogue over `sets`, the first being the core rules.
    #[must_use]
    pub fn from_sets(sets: Vec<RuleSet>) -> Self {
        Self {
            sets: sets.into_iter().map(Arc::new).collect(),
        }
    }

    /// The rule sets, core first, shared: every engine of a run takes them from here, so the
    /// packs load and compile once per run.
    #[must_use]
    pub fn shared_sets(&self) -> Vec<Arc<RuleSet>> {
        self.sets.clone()
    }

    /// Every category, core first.
    pub fn categories(&self) -> impl Iterator<Item = &LoadedCategory> {
        self.sets.iter().flat_map(|s| s.categories().iter())
    }

    /// Every rule, core first.
    pub fn rules(&self) -> impl Iterator<Item = &LoadedRule> {
        self.sets.iter().flat_map(|s| s.rules())
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
