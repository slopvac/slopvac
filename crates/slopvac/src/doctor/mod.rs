//! `slopvac doctor`: a read-only report on the binary, its configuration and its local state.
//!
//! The JSON shape is `schemas/v3/doctor-report.schema.json`; docs/contract/doctor.md specifies
//! it. Doctor opens no connection and changes no file. Data owned by other beads comes through
//! the traits below; [`stubs`] holds the local stand-ins.

pub mod read;
pub mod stubs;

use std::path::{Path, PathBuf};

use serde::Serialize;
use slopvac_plugins::lock::{Lockfile, PackKind};
use slopvac_plugins::sync::{PackState, Store, status};
use slopvac_plugins::trust::TrustStore;

use crate::env::Env;
use crate::redact::path_for_log;
use read::{ReadProblem, read_bounded};

/// Largest lockfile doctor reads, as the lock reader's own cap.
const MAX_LOCK_BYTES: u64 = slopvac_plugins::lock::MAX_LOCK_BYTES as u64;
/// Most entries the directory size walk visits.
const WALK_LIMIT: usize = 10_000;

/// What the update cache and the install say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFacts {
    /// Stable id of the install method.
    pub install_method: &'static str,
    /// The command that upgrades this install.
    pub upgrade_command: Option<String>,
    /// Why update checks are off, if they are.
    pub disabled_by: Option<&'static str>,
    /// The cached latest release and the unix time it was checked.
    pub cached_latest: Option<(String, u64)>,
}

/// Source of [`UpdateFacts`].
pub trait UpdateProbe {
    /// Read the facts. Reads local files only.
    fn facts(
        &self,
        env: &dyn Env,
        offline: bool,
        config_update_check: Option<bool>,
        cache_dir: Option<&Path>,
    ) -> UpdateFacts;
}

/// One status of a config source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceStatus {
    /// Parses and declares `schema_version` 3.
    Ok,
    /// A 2.x file.
    Legacy,
    /// The file does not exist.
    Missing,
    /// The file could not be read.
    Unreadable,
    /// The file is not valid.
    Invalid,
}

/// A config file that was found, before path redaction.
#[derive(Debug, Clone)]
pub struct RawSource {
    /// `explicit` or `project`.
    pub layer: &'static str,
    /// Where the file is.
    pub path: PathBuf,
    /// What reading it showed.
    pub status: SourceStatus,
    /// A message with no path, when the status is not ok.
    pub error: Option<String>,
}

/// What a config scan found.
#[derive(Debug, Clone, Default)]
pub struct ConfigScan {
    /// Sources in layer order.
    pub sources: Vec<RawSource>,
    /// `[network] offline = true` in a source.
    pub offline: bool,
    /// `[network] update_check` in a source.
    pub update_check: Option<bool>,
}

/// Source of [`ConfigScan`].
pub trait ConfigSources {
    /// Find the config sources for `cwd`, or the one `--config` file.
    fn scan(&self, cwd: &Path, explicit: Option<&Path>) -> ConfigScan;
}

/// A per-user directory and how it was chosen.
#[derive(Debug, Clone)]
pub struct Located {
    /// The directory, or `None` when the environment gives no home.
    pub path: Option<PathBuf>,
    /// An environment variable name, `default` or `unresolved`.
    pub source: String,
}

impl Located {
    /// A resolved directory.
    #[must_use]
    pub fn at(path: PathBuf, source: &str) -> Self {
        Self {
            path: Some(path),
            source: source.to_owned(),
        }
    }

    /// No directory could be derived.
    #[must_use]
    pub fn unresolved() -> Self {
        Self {
            path: None,
            source: "unresolved".to_owned(),
        }
    }
}

/// Source of the per-user directories.
pub trait Dirs {
    /// The cache directory.
    fn cache(&self, env: &dyn Env) -> Located;
    /// The data directory, which is the pack store root.
    fn data(&self, env: &dyn Env) -> Located;
    /// The state directory, which holds the trust store.
    fn state(&self, env: &dyn Env) -> Located;
}

/// Everything a report needs from outside.
pub struct Inputs<'a> {
    /// Environment access.
    pub env: &'a dyn Env,
    /// Working directory. Printed paths are relative to it.
    pub cwd: &'a Path,
    /// `--offline` was given.
    pub offline_flag: bool,
    /// `--config FILE`.
    pub explicit_config: Option<&'a Path>,
    /// The running version.
    pub version: &'a str,
    /// Update facts.
    pub update: &'a dyn UpdateProbe,
    /// Config discovery.
    pub config: &'a dyn ConfigSources,
    /// Per-user directories.
    pub dirs: &'a dyn Dirs,
}

/// The report. Field order is the JSON key order.
#[derive(Debug, Serialize)]
pub struct Report {
    schema_version: u8,
    /// Nothing needs attention.
    pub ok: bool,
    slopvac: Binary,
    platform: Platform,
    update: Update,
    offline: Offline,
    config: Config,
    packs: Packs,
    trust: Trust,
    directories: Directories,
}

#[derive(Debug, Serialize)]
struct Binary {
    version: String,
    install_method: &'static str,
    upgrade_command: Option<String>,
}

#[derive(Debug, Serialize)]
struct Platform {
    os: &'static str,
    arch: &'static str,
}

#[derive(Debug, Serialize)]
struct Update {
    checks: &'static str,
    disabled_by: Option<&'static str>,
    state: &'static str,
    latest: Option<String>,
    checked_at: Option<u64>,
}

#[derive(Debug, Serialize)]
struct Offline {
    effective: bool,
    sources: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct ConfigSource {
    layer: &'static str,
    file: String,
    status: SourceStatus,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct Config {
    ok: bool,
    sources: Vec<ConfigSource>,
}

#[derive(Debug, Serialize)]
struct PackEntry {
    id: String,
    version: String,
    kind: &'static str,
    state: &'static str,
}

#[derive(Debug, Serialize)]
struct Lock {
    file: String,
    status: &'static str,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct Packs {
    lockfile: Lock,
    items: Vec<PackEntry>,
}

#[derive(Debug, Serialize)]
struct PinnedKey {
    namespace: String,
    fingerprint: String,
}

#[derive(Debug, Serialize)]
struct Trust {
    status: &'static str,
    error: Option<String>,
    trusted_plugins: usize,
    pinned_keys: Vec<PinnedKey>,
}

#[derive(Debug, Serialize)]
struct Directory {
    path: String,
    source: String,
    exists: bool,
    bytes: Option<u64>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct Directories {
    cache: Directory,
    data: Directory,
    state: Directory,
}

fn directory(located: &Located, cwd: &Path) -> Directory {
    let Some(path) = &located.path else {
        return Directory {
            path: "<unresolved>".to_owned(),
            source: located.source.clone(),
            exists: false,
            bytes: None,
            truncated: false,
        };
    };
    let exists = path.is_dir();
    let (bytes, truncated) = if exists {
        let (b, t) = walk_size(path);
        (Some(b), t)
    } else {
        (None, false)
    };
    Directory {
        path: path_for_log(path, cwd),
        source: located.source.clone(),
        exists,
        bytes,
        truncated,
    }
}

/// Total size of regular files below `root`, visiting at most [`WALK_LIMIT`] entries. Links are
/// not followed.
fn walk_size(root: &Path) -> (u64, bool) {
    let mut total = 0u64;
    let mut visited = 0usize;
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > WALK_LIMIT {
                return (total, true);
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    (total, false)
}

fn pack_state(state: &PackState) -> &'static str {
    match state {
        PackState::NotInstalled => "not-installed",
        PackState::Recorded(_) => "recorded",
        PackState::Tampered(_) => "tampered",
        PackState::InPlace(_) => "in-place",
        PackState::InPlaceChanged(_) => "changed",
    }
}

fn packs_report(project_root: &Path, cwd: &Path, store_root: Option<&Path>) -> Packs {
    let lock_path = project_root.join(".slopvac").join("packs.lock");
    let file = path_for_log(&lock_path, cwd);
    let lock = |status, error: Option<String>| Lock {
        file: file.clone(),
        status,
        error,
    };
    let empty = |lockfile| Packs {
        lockfile,
        items: Vec::new(),
    };
    let text = match read_bounded(&lock_path, MAX_LOCK_BYTES) {
        Ok(Some(t)) => t,
        Ok(None) => return empty(lock("absent", None)),
        Err(problem) => {
            let why = match problem {
                ReadProblem::NotRegular => "not a regular file",
                ReadProblem::TooLarge => "larger than the lockfile limit",
                ReadProblem::Io => "cannot read the file",
            };
            return empty(lock("unreadable", Some(why.to_owned())));
        }
    };
    let parsed = match Lockfile::parse(&text) {
        Ok(l) => l,
        Err(e) => return empty(lock("invalid", Some(e.to_string()))),
    };
    let Some(root) = store_root else {
        return empty(lock(
            "unreadable",
            Some("pack store: no data directory could be derived".to_owned()),
        ));
    };
    match status(&Store::new(root), project_root, &parsed) {
        Ok(statuses) => Packs {
            lockfile: lock("ok", None),
            items: parsed
                .packs()
                .iter()
                .zip(statuses)
                .map(|(pack, s)| PackEntry {
                    id: pack.id.to_string(),
                    version: pack.version.to_string(),
                    kind: match pack.kind {
                        PackKind::Data => "data",
                        PackKind::Plugin => "plugin",
                    },
                    state: pack_state(&s.state),
                })
                .collect(),
        },
        Err(_) => empty(lock(
            "unreadable",
            Some("pack store: cannot read the installed packs".to_owned()),
        )),
    }
}

fn trust_report(state_dir: Option<&Path>) -> Trust {
    let none = |status, error: Option<&str>| Trust {
        status,
        error: error.map(str::to_owned),
        trusted_plugins: 0,
        pinned_keys: Vec::new(),
    };
    let Some(dir) = state_dir else {
        return none("absent", None);
    };
    // The file name is the trust store's own (`trust.json`); the store has no existence query.
    if !dir.join("trust.json").exists() {
        return none("absent", None);
    }
    let store = TrustStore::open(dir);
    let unreadable = || {
        none(
            "unreadable",
            Some("the trust store is damaged or unreadable"),
        )
    };
    let Ok(records) = store.records() else {
        return unreadable();
    };
    let Ok(pins) = store.pinned_map() else {
        return unreadable();
    };
    Trust {
        status: "ok",
        error: None,
        trusted_plugins: records.len(),
        pinned_keys: pins
            .iter()
            .flat_map(|(namespace, keys)| {
                keys.iter().map(|k| PinnedKey {
                    namespace: namespace.clone(),
                    fingerprint: k.fingerprint(),
                })
            })
            .collect(),
    }
}

/// Build the report.
#[must_use]
pub fn build(i: &Inputs<'_>) -> Report {
    let scan = i.config.scan(i.cwd, i.explicit_config);

    let mut offline_sources = Vec::new();
    if i.offline_flag {
        offline_sources.push("flag");
    }
    if crate::offline::env_requests_offline(i.env) {
        offline_sources.push("env");
    }
    if scan.offline {
        offline_sources.push("config");
    }
    let offline_on = !offline_sources.is_empty();

    let cache = i.dirs.cache(i.env);
    let data = i.dirs.data(i.env);
    let state = i.dirs.state(i.env);

    let facts = i
        .update
        .facts(i.env, offline_on, scan.update_check, cache.path.as_deref());
    let (update_state, latest, checked_at) = match &facts.cached_latest {
        Some((latest, at)) => {
            let newer = semver::Version::parse(latest)
                .ok()
                .zip(semver::Version::parse(i.version).ok())
                .is_some_and(|(l, c)| l > c);
            (
                if newer { "available" } else { "current" },
                Some(latest.clone()),
                Some(*at),
            )
        }
        None => ("unknown", None, None),
    };

    let project_root = scan
        .sources
        .first()
        .and_then(|s| s.path.parent())
        .map_or_else(|| i.cwd.to_owned(), Path::to_owned);

    let packs = packs_report(&project_root, i.cwd, data.path.as_deref());
    let trust = trust_report(state.path.as_deref());

    let config = Config {
        ok: scan.sources.iter().all(|s| s.status == SourceStatus::Ok),
        sources: scan
            .sources
            .iter()
            .map(|s| ConfigSource {
                layer: s.layer,
                file: path_for_log(&s.path, i.cwd),
                status: s.status,
                error: s.error.clone(),
            })
            .collect(),
    };

    let ok = config.ok
        && !matches!(packs.lockfile.status, "invalid" | "unreadable")
        && packs.items.iter().all(|p| p.state != "tampered")
        && trust.status != "unreadable";

    Report {
        schema_version: 3,
        ok,
        slopvac: Binary {
            version: i.version.to_owned(),
            install_method: facts.install_method,
            upgrade_command: facts.upgrade_command,
        },
        platform: Platform {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        },
        update: Update {
            checks: if facts.disabled_by.is_some() {
                "disabled"
            } else {
                "enabled"
            },
            disabled_by: facts.disabled_by,
            state: update_state,
            latest,
            checked_at,
        },
        offline: Offline {
            effective: offline_on,
            sources: offline_sources,
        },
        config,
        packs,
        trust,
        directories: Directories {
            cache: directory(&cache, i.cwd),
            data: directory(&data, i.cwd),
            state: directory(&state, i.cwd),
        },
    }
}

impl Report {
    /// Pretty JSON with a trailing newline.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("the report serialises");
        text.push('\n');
        text
    }

    /// A short human summary, one fact per line.
    #[must_use]
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "slopvac {} ({}) on {}/{}",
            self.slopvac.version, self.slopvac.install_method, self.platform.os, self.platform.arch
        );
        let _ = match (&self.update.latest, self.update.state) {
            (Some(latest), "available") => {
                let hint = self
                    .slopvac
                    .upgrade_command
                    .as_deref()
                    .map_or(String::new(), |c| format!(" Upgrade with `{c}`."));
                writeln!(out, "update: {latest} is available.{hint}")
            }
            (Some(latest), _) => writeln!(out, "update: up to date (latest known {latest})"),
            (None, _) => writeln!(out, "update: no cached check"),
        };
        if let Some(why) = self.update.disabled_by {
            let _ = writeln!(out, "update checks: off ({why})");
        }
        if self.offline.effective {
            let _ = writeln!(out, "offline: on ({})", self.offline.sources.join(", "));
        }
        if self.config.sources.is_empty() {
            let _ = writeln!(out, "config: none found, defaults apply");
        }
        for s in &self.config.sources {
            let detail = s
                .error
                .as_deref()
                .map_or(String::new(), |e| format!(": {e}"));
            let status = serde_json::to_value(s.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            let _ = writeln!(out, "config {}: {} {status}{detail}", s.layer, s.file);
        }
        let lock = &self.packs.lockfile;
        let detail = lock
            .error
            .as_deref()
            .map_or(String::new(), |e| format!(": {e}"));
        let _ = writeln!(out, "packs: {} {}{detail}", lock.file, lock.status);
        for p in &self.packs.items {
            let _ = writeln!(out, "  {} {} ({}) {}", p.id, p.version, p.kind, p.state);
        }
        let _ = writeln!(
            out,
            "trust: {} ({} plugin approvals, {} pinned keys)",
            self.trust.status,
            self.trust.trusted_plugins,
            self.trust.pinned_keys.len()
        );
        for (name, d) in [
            ("cache", &self.directories.cache),
            ("data", &self.directories.data),
            ("state", &self.directories.state),
        ] {
            let size = d.bytes.map_or(String::new(), |b| {
                format!(", {b} bytes{}", if d.truncated { "+" } else { "" })
            });
            let _ = writeln!(
                out,
                "{name} dir: {} ({}{})",
                d.path,
                if d.exists { "exists" } else { "absent" },
                size
            );
        }
        let _ = writeln!(out, "{}", if self.ok { "ok" } else { "problems found" });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    struct Fixed(Option<(String, u64)>);

    impl UpdateProbe for Fixed {
        fn facts(&self, _: &dyn Env, _: bool, _: Option<bool>, _: Option<&Path>) -> UpdateFacts {
            UpdateFacts {
                install_method: "homebrew",
                upgrade_command: Some("brew upgrade srobroek/tap/slopvac".into()),
                disabled_by: None,
                cached_latest: self.0.clone(),
            }
        }
    }

    fn report(version: &str, cached: Option<&str>) -> Report {
        let env = MapEnv::default();
        build(&Inputs {
            env: &env,
            cwd: Path::new("/nonexistent-cwd"),
            offline_flag: false,
            explicit_config: None,
            version,
            update: &Fixed(cached.map(|v| (v.to_owned(), 7))),
            config: &stubs::LocalConfigSources,
            dirs: &stubs::EnvDirs,
        })
    }

    #[test]
    fn update_state_compares_semantic_versions_not_strings() {
        // "3.10.0" sorts before "3.9.0" as text but is newer.
        for (current, cached, state) in [
            ("3.9.0", "3.10.0", "available"),
            ("3.10.0", "3.9.0", "current"),
            ("3.0.0", "3.0.0", "current"),
            ("3.0.0-rc.1", "3.0.0", "available"),
            ("3.0.0", "garbage", "current"),
        ] {
            let r = report(current, Some(cached));
            assert_eq!(r.update.state, state, "{current} vs {cached}");
        }
        assert_eq!(report("3.0.0", None).update.state, "unknown");
    }

    #[test]
    fn the_text_summary_names_the_upgrade_command_for_an_available_update() {
        let text = report("3.0.0", Some("3.1.0")).to_text();
        assert!(
            text.contains(
                "update: 3.1.0 is available. Upgrade with `brew upgrade srobroek/tap/slopvac`."
            ),
            "{text}"
        );
    }
}
