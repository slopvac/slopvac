//! Local stand-ins for the data other beads own. Doctor codes against the traits in
//! [`super`]; these implementations fake the owners until they land.
//!
//! - [`LocalConfigSources`]: STUB(slopvac-8l4.12.23). `slopvac-config` (slopvac-8l4.4.2) owns
//!   layered config. This finds one `slopvac.toml`, checks `schema_version`, and reads
//!   `[network] offline` and `update_check`.
//! - [`EnvDirs`]: STUB(slopvac-8l4.12.24). The plugin beads own the per-user directories. This
//!   derives them from the environment and treats the data directory as the pack store root and
//!   the state directory as the trust store.

use std::path::{Path, PathBuf};

use super::read::{ReadProblem, read_bounded};
use super::{ConfigScan, ConfigSources, Dirs, Located, RawSource, SourceStatus};
use crate::env::Env;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Finds the nearest `slopvac.toml` (or the `--config` file) and checks its version stamp.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalConfigSources;

impl ConfigSources for LocalConfigSources {
    fn scan(&self, cwd: &Path, explicit: Option<&Path>) -> ConfigScan {
        let (layer, path) = match explicit {
            Some(p) => ("explicit", Some(cwd.join(p))),
            None => ("project", nearest(cwd)),
        };
        let Some(path) = path else {
            return ConfigScan::default();
        };
        let (status, error, network) = inspect(&path);
        ConfigScan {
            sources: vec![RawSource {
                layer,
                path,
                status,
                error,
            }],
            offline: network.0,
            update_check: network.1,
        }
    }
}

/// The nearest `slopvac.toml`, walking up and stopping after the directory that holds `.git`.
fn nearest(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let file = dir.join("slopvac.toml");
        // A directory or FIFO of that name still counts: `inspect` reports it.
        if std::fs::symlink_metadata(&file).is_ok() {
            return Some(file);
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    None
}

type Network = (bool, Option<bool>);

fn inspect(path: &Path) -> (SourceStatus, Option<String>, Network) {
    let text = match read_bounded(path, MAX_CONFIG_BYTES) {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                SourceStatus::Missing,
                Some("file not found".into()),
                (false, None),
            );
        }
        Err(ReadProblem::NotRegular) => {
            return (
                SourceStatus::Unreadable,
                Some("not a regular file".into()),
                (false, None),
            );
        }
        Err(ReadProblem::TooLarge) => {
            return (
                SourceStatus::Unreadable,
                Some(format!("larger than {MAX_CONFIG_BYTES} bytes")),
                (false, None),
            );
        }
        Err(ReadProblem::Io) => {
            return (
                SourceStatus::Unreadable,
                Some("cannot read the file".into()),
                (false, None),
            );
        }
    };
    let table = match text.parse::<toml::Table>() {
        Ok(t) => t,
        Err(e) => {
            let line = e.span().map_or(0, |s| {
                text[..s.start.min(text.len())].matches('\n').count() + 1
            });
            let message = if line > 0 {
                format!("not valid TOML (line {line})")
            } else {
                "not valid TOML".to_owned()
            };
            return (SourceStatus::Invalid, Some(message), (false, None));
        }
    };
    let network = table.get("network").and_then(toml::Value::as_table);
    let net = (
        network
            .and_then(|n| n.get("offline"))
            .and_then(toml::Value::as_bool)
            == Some(true),
        network
            .and_then(|n| n.get("update_check"))
            .and_then(toml::Value::as_bool),
    );
    match table
        .get("schema_version")
        .and_then(toml::Value::as_integer)
    {
        Some(3) => (SourceStatus::Ok, None, net),
        None | Some(2) => (
            SourceStatus::Legacy,
            Some("2.x configuration (schema_version is not 3): run `slopvac migrate`".into()),
            net,
        ),
        Some(_) => (
            SourceStatus::Invalid,
            Some("unsupported schema_version".into()),
            net,
        ),
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
