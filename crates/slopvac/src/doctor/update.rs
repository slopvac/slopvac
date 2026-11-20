//! The update and install facts doctor reports, read from `slopvac-update`.
//!
//! Doctor never fetches: the install method comes from the running executable and the install
//! receipt, and the latest release comes from the cache file that the update check writes.

use std::path::{Path, PathBuf};

use slopvac_update::install::{InstallMethod, detect};
use slopvac_update::notice::Cache;

use super::{UpdateFacts, UpdateProbe};
use crate::env::Env;

/// Detects the install channel of the running binary and reads the cached update answer.
#[derive(Debug, Clone, Default)]
pub struct InstalledUpdateProbe {
    /// The executable to inspect; the running one when `None`.
    exe: Option<PathBuf>,
}

impl InstalledUpdateProbe {
    /// A probe for the running executable.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn method(&self, env: &dyn Env) -> InstallMethod {
        let exe = self
            .exe
            .clone()
            .or_else(|| std::env::current_exe().ok())
            .and_then(|p| std::fs::canonicalize(p).ok());
        exe.map_or(InstallMethod::Unknown, |exe| {
            detect(&exe, &|name| env.var(name))
        })
    }
}

/// Why update checks are off, if they are, in the order the doctor spec gives.
fn disabled_by(
    env: &dyn Env,
    offline: bool,
    config_update_check: Option<bool>,
) -> Option<&'static str> {
    let set = |n: &str| env.var(n).is_some_and(|v| !v.is_empty());
    if offline {
        Some("offline")
    } else if env.var("CI").is_some() {
        Some("ci")
    } else if [
        "NO_UPDATE_NOTIFIER",
        "SLOPVAC_NO_UPDATE_CHECK",
        "DO_NOT_TRACK",
    ]
    .iter()
    .any(|n| set(n))
    {
        Some("env")
    } else if config_update_check == Some(false) {
        Some("config")
    } else {
        None
    }
}

/// The cached answer. `slopvac-update` reads the file without blocking and with a size cap.
fn cached(dir: &Path) -> Option<(String, u64)> {
    let entry = Cache::at(dir.join("update-check.json")).read()?;
    Some((entry.latest, entry.checked_at))
}

impl UpdateProbe for InstalledUpdateProbe {
    fn facts(
        &self,
        env: &dyn Env,
        offline: bool,
        config_update_check: Option<bool>,
        cache_dir: Option<&Path>,
    ) -> UpdateFacts {
        let method = self.method(env);
        UpdateFacts {
            install_method: method.id(),
            upgrade_command: method.upgrade_command().map(str::to_owned),
            disabled_by: disabled_by(env, offline, config_update_check),
            cached_latest: cache_dir.and_then(cached),
        }
    }
}
