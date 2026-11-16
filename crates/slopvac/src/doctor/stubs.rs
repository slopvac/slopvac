//! Local stand-ins for the data other beads own. Doctor codes against the traits in
//! [`super`]; these implementations fake the owners until they land.
//!
//! - [`LocalUpdateProbe`]: STUB(slopvac-8l4.12.22). `slopvac-update` (slopvac-8l4.12.4) owns
//!   install-method detection and the update cache. This reads the cache file only and reports the
//!   install method as `unknown`.
//! - [`EnvDirs`]: STUB(slopvac-8l4.12.24). The plugin beads own the per-user directories. This
//!   derives them from the environment and treats the data directory as the pack store root and
//!   the state directory as the trust store.

use std::path::{Path, PathBuf};

use super::read::read_bounded;
use super::{Dirs, Located, UpdateFacts, UpdateProbe};
use crate::env::Env;

const MAX_CACHE_BYTES: u64 = 64 * 1024;

/// Reads the 24-hour update cache; knows no install method.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalUpdateProbe;

impl UpdateProbe for LocalUpdateProbe {
    fn facts(
        &self,
        env: &dyn Env,
        offline: bool,
        config_update_check: Option<bool>,
        cache_dir: Option<&Path>,
    ) -> UpdateFacts {
        let set = |n: &str| env.var(n).is_some_and(|v| !v.is_empty());
        let disabled_by = if offline {
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
        };
        let cached = cache_dir
            .and_then(|d| read_bounded(&d.join("update-check.json"), MAX_CACHE_BYTES).ok()?)
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| {
                Some((
                    v.get("latest")?.as_str()?.to_owned(),
                    v.get("checked_at")?.as_u64()?,
                ))
            });
        UpdateFacts {
            install_method: "unknown",
            upgrade_command: None,
            disabled_by,
            cached_latest: cached,
        }
    }
}

/// Per-user directories from the environment: XDG on Unix, `LOCALAPPDATA` on Windows.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvDirs;

impl EnvDirs {
    fn locate(
        env: &dyn Env,
        override_var: Option<&str>,
        xdg_var: &str,
        home_fallback: &str,
        windows_leaf: &str,
    ) -> Located {
        let get = |n: &str| env.var(n).filter(|v| !v.is_empty());
        if let Some(name) = override_var
            && let Some(v) = get(name)
        {
            return Located::at(PathBuf::from(v), name);
        }
        if cfg!(windows) {
            return get("LOCALAPPDATA").map_or_else(Located::unresolved, |v| {
                Located::at(
                    PathBuf::from(v).join("slopvac").join(windows_leaf),
                    "default",
                )
            });
        }
        if let Some(v) = get(xdg_var) {
            return Located::at(PathBuf::from(v).join("slopvac"), xdg_var);
        }
        get("HOME").map_or_else(Located::unresolved, |h| {
            Located::at(
                PathBuf::from(h).join(home_fallback).join("slopvac"),
                "default",
            )
        })
    }
}

impl Dirs for EnvDirs {
    fn cache(&self, env: &dyn Env) -> Located {
        Self::locate(
            env,
            Some("SLOPVAC_CACHE_DIR"),
            "XDG_CACHE_HOME",
            ".cache",
            "cache",
        )
    }

    fn data(&self, env: &dyn Env) -> Located {
        Self::locate(env, None, "XDG_DATA_HOME", ".local/share", "data")
    }

    fn state(&self, env: &dyn Env) -> Located {
        Self::locate(env, None, "XDG_STATE_HOME", ".local/state", "state")
    }
}
