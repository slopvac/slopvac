//! `slopvac doctor`: a read-only report on the binary, its configuration and its local state.
//!
//! The JSON shape is `schemas/v3/doctor-report.schema.json`; docs/contract/doctor.md specifies
//! it. Doctor opens no connection and changes no file. Data owned by other beads comes through
//! the traits below; [`stubs`] holds the local stand-ins.

pub mod dirs;
pub mod read;
mod schema;
pub mod stubs;
pub mod update;

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Serialize;
use slopvac_model::x_extensions;
use slopvac_plugins::lock::{LockError, Lockfile, PackKind};
use slopvac_plugins::sign::PublicKey;
use slopvac_plugins::sync::{PackState, Store, status};
use slopvac_plugins::trust::{MAX_STORE_FILE_BYTES, parse_store};

use crate::env::Env;
use crate::redact::path_for_log as redacted_path;
use read::{ReadProblem, read_bounded};

/// A path safe to print (policies.md, section 1.7), with `/` as the separator on every
/// platform, so the report reads the same everywhere.
fn path_for_log(path: &Path, cwd: &Path) -> String {
    let shown = redacted_path(path, cwd);
    if cfg!(windows) {
        shown.replace('\\', "/")
    } else {
        shown
    }
}

/// Largest lockfile doctor reads, as the lock reader's own cap.
const MAX_LOCK_BYTES: u64 = slopvac_plugins::lock::MAX_LOCK_BYTES as u64;
/// Most entries the directory size walk visits. The schema and the spec say 10000.
pub const WALK_LIMIT: usize = 10_000;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline, transform = schema::extensible)]
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
    /// Most entries the size walk of a directory visits. Use [`WALK_LIMIT`].
    pub walk_limit: usize,
}

/// The only `schema_version` of the doctor report.
pub const SCHEMA_VERSION: u8 = 3;

/// The report. Field order is the JSON key order.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct Report {
    // The shared definition, not the Rust integer range.
    #[schemars(schema_with = "schema::schema_version")]
    schema_version: u8,
    /// True when nothing needs attention: configuration is valid, the lockfile parses, no plugin
    /// is tampered and the trust store is readable. The exit code is 0 when true and 2 when
    /// false. An available update, a missing lockfile or an empty trust store do not make it
    /// false.
    pub ok: bool,
    slopvac: Binary,
    platform: Platform,
    update: Update,
    offline: Offline,
    config: Config,
    plugins: Packs,
    trust: Trust,
    directories: Directories,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Binary {
    /// The running version.
    #[schemars(length(min = 1))]
    version: String,
    /// How this binary was installed. `unknown` when no channel matches.
    #[schemars(schema_with = "schema::install_method")]
    install_method: &'static str,
    /// The command that upgrades this install, or null when the channel is unknown.
    upgrade_command: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Platform {
    /// Operating system, as Rust reports it (`linux`, `macos`, `windows`).
    #[schemars(length(min = 1))]
    os: &'static str,
    /// CPU architecture, as Rust reports it (`x86_64`, `aarch64`).
    #[schemars(length(min = 1))]
    arch: &'static str,
}

/// What the cached update check says. Doctor reads the cache and starts no request in this
/// build.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Update {
    /// `enabled` or `disabled`. Disabled when `disabled_by` is set.
    #[schemars(schema_with = "schema::update_checks")]
    checks: &'static str,
    /// Why checks are off: offline mode, the CI variable, an opt-out variable, or
    /// `[network] update_check = false`. Null when enabled.
    #[schemars(schema_with = "schema::update_disabled_by")]
    disabled_by: Option<&'static str>,
    /// `available` when the cached latest release is newer than the running version, `current`
    /// when it is not, `unknown` when no cached answer exists or the cached version is not a
    /// semantic version.
    #[schemars(schema_with = "schema::update_state")]
    state: &'static str,
    /// The cached latest release version, or null.
    latest: Option<String>,
    /// Unix seconds of the cached check, or null.
    checked_at: Option<u64>,
}

/// Sticky offline mode (policies.md, section 1.3).
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Offline {
    effective: bool,
    /// Every source that turned offline on, in the order flag, env, config.
    #[schemars(schema_with = "schema::offline_sources")]
    sources: Vec<&'static str>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct DoctorConfigFile {
    /// `explicit` for a file named with `--config`, `project` for the nearest `slopvac.toml`.
    #[schemars(schema_with = "schema::config_layer")]
    layer: &'static str,
    #[schemars(with = "schema::PathText")]
    file: String,
    /// `ok` when the file parses and declares `schema_version` 3. `legacy` for a 2.x file (no
    /// `schema_version` or 2). `missing`, `unreadable` or `invalid` otherwise.
    status: SourceStatus,
    #[schemars(schema_with = "schema::nullable_message")]
    error: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Config {
    /// True when every listed source has status `ok`. True with no sources: the defaults apply.
    ok: bool,
    sources: Vec<DoctorConfigFile>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(rename = "LockedPlugin", transform = x_extensions)]
struct PackEntry {
    #[schemars(length(min = 1))]
    id: String,
    #[schemars(length(min = 1))]
    version: String,
    #[schemars(schema_with = "schema::plugin_kind")]
    kind: &'static str,
    /// `recorded`: installed and intact. `in-place`: a project directory that matches the lock.
    /// `changed`: a project directory that differs from the lock. `tampered`: installed but
    /// differs from its record. `not-installed`: absent from the store.
    #[schemars(schema_with = "schema::plugin_state")]
    state: &'static str,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(inline, transform = x_extensions)]
struct Lock {
    #[schemars(with = "schema::PathText")]
    file: String,
    /// `absent` is not an error: a project need not use plugins.
    #[schemars(schema_with = "schema::lockfile_status")]
    status: &'static str,
    #[schemars(schema_with = "schema::nullable_message")]
    error: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(rename = "Plugins", transform = x_extensions)]
struct Packs {
    lockfile: Lock,
    /// One entry per locked plugin, sorted by id. Empty unless the lockfile status is `ok`.
    items: Vec<PackEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct PinnedKey {
    /// The pinned namespace, in the plugin id pattern. Doctor reports a store that holds any
    /// other text as unreadable.
    #[schemars(extend("pattern" = r"^[a-z][a-z0-9-]{0,63}(?![\s\S])"))]
    namespace: String,
    /// SHA-256 of the public key, 64 lowercase hex digits, derived by doctor from the stored
    /// key. The key itself is never printed.
    #[schemars(extend("pattern" = r"^[0-9a-f]{64}(?![\s\S])"))]
    fingerprint: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Trust {
    /// `absent` when no trust file exists yet.
    #[schemars(schema_with = "schema::trust_status")]
    status: &'static str,
    #[schemars(schema_with = "schema::nullable_message")]
    error: Option<String>,
    /// Number of recorded plugin approvals.
    trusted_plugins: usize,
    pinned_keys: Vec<PinnedKey>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Directory {
    #[schemars(with = "schema::PathText")]
    path: String,
    /// How the location was chosen: an environment variable name, or `default`.
    #[schemars(length(min = 1))]
    source: String,
    exists: bool,
    /// Total size of regular files found, or null when the directory does not exist. Counts at
    /// most 10000 entries.
    bytes: Option<u64>,
    /// True when the walk stopped at the entry limit, so `bytes` is a lower bound.
    truncated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = x_extensions)]
struct Directories {
    cache: Directory,
    data: Directory,
    state: Directory,
}

fn directory(located: &Located, cwd: &Path, walk_limit: usize) -> Directory {
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
        let (b, t) = walk_size(path, walk_limit);
        (Some(b), t)
    } else {
        (None, false)
    };
    let shown = path_for_log(path, cwd);
    Directory {
        // The working directory itself is the empty relative path; the schema wants a name.
        path: if shown.is_empty() {
            ".".to_owned()
        } else {
            shown
        },
        source: located.source.clone(),
        exists,
        bytes,
        truncated,
    }
}

/// Total size of regular files below `root`, visiting at most `limit` entries. Links are not
/// followed. The flag is true when the limit stopped the walk.
fn walk_size(root: &Path, limit: usize) -> (u64, bool) {
    let mut total = 0u64;
    let mut visited = 0usize;
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > limit {
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

/// Keys a lockfile may hold. A pointer segment outside this set came from the file, so it is
/// shown as `*`.
const LOCK_KEYS: [&str; 12] = [
    "schema_version",
    "plugins",
    "id",
    "kind",
    "version",
    "slopvac_api",
    "source",
    "digest",
    "signature",
    "key_id",
    "not_before",
    "requires",
];

/// Where a lockfile is wrong, with nothing the file said. The parser's own message can echo the
/// rejected value, so it is dropped (CWE-209). Only the location survives, and only the segments
/// that are known keys or array indexes.
fn lock_diagnostic(e: &LockError) -> String {
    if e.pointer.is_empty() {
        return "not a valid lockfile".to_owned();
    }
    let shown: Vec<&str> = e
        .pointer
        .split('/')
        .skip(1)
        .map(|seg| {
            if LOCK_KEYS.contains(&seg)
                || (!seg.is_empty() && seg.len() <= 6 && seg.bytes().all(|b| b.is_ascii_digit()))
            {
                seg
            } else {
                "*"
            }
        })
        .collect();
    format!("invalid value at /{}", shown.join("/"))
}

fn packs_report(project_root: &Path, cwd: &Path, store_root: Option<&Path>) -> Packs {
    let lock_path = project_root.join(".slopvac").join("plugins.lock");
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
        Err(e) => return empty(lock("invalid", Some(lock_diagnostic(&e)))),
    };
    let Some(root) = store_root else {
        return empty(lock(
            "unreadable",
            Some("plugin store: no data directory could be derived".to_owned()),
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
                        PackKind::Code => "code",
                    },
                    state: pack_state(&s.state),
                })
                .collect(),
        },
        Err(_) => empty(lock(
            "unreadable",
            Some("plugin store: cannot read the installed plugins".to_owned()),
        )),
    }
}

/// A pinned namespace: the pack id pattern, `^[a-z][a-z0-9-]{0,63}$`.
fn is_namespace(s: &str) -> bool {
    let mut chars = s.chars();
    s.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
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
    // The file name is the trust store's own (`trust.json`). The bytes come through doctor's
    // bounded reader: a FIFO or device cannot hang it, and a permission error is not "absent".
    let path = dir.join("trust.json");
    let text = match read_bounded(&path, MAX_STORE_FILE_BYTES) {
        Ok(None) => return none("absent", None),
        Ok(Some(t)) => t,
        Err(problem) => {
            let why = match problem {
                ReadProblem::NotRegular => "the trust store is not a regular file",
                ReadProblem::TooLarge => "the trust store is larger than the reader accepts",
                ReadProblem::Io => "the trust store cannot be read",
            };
            return none("unreadable", Some(why));
        }
    };
    let Ok(snapshot) = parse_store(text.as_bytes(), &path) else {
        return none("unreadable", Some("the trust store is damaged"));
    };
    let invalid_pin = || {
        none(
            "unreadable",
            Some("the trust store holds an invalid pinned key"),
        )
    };
    // Nothing stored is printed as it stands. The key must parse, the fingerprint is derived from
    // it (a stored one must agree), and the namespace must look like a pack id, so a hand-edited
    // file cannot put arbitrary text into the report (CWE-209).
    let mut pinned = Vec::new();
    for p in snapshot.pins {
        let Ok(key) = PublicKey::from_base64(&p.key) else {
            return invalid_pin();
        };
        let fingerprint = key.fingerprint();
        if fingerprint != p.fingerprint || !is_namespace(&p.namespace) {
            return invalid_pin();
        }
        pinned.push(PinnedKey {
            namespace: p.namespace,
            fingerprint,
        });
    }
    pinned.sort_by(|a, b| a.namespace.cmp(&b.namespace));
    Trust {
        status: "ok",
        error: None,
        trusted_plugins: snapshot.records.len(),
        pinned_keys: pinned,
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
    // A cached answer that is not a semantic version cannot be compared, so it counts as no
    // answer, and the text is not echoed.
    let cached = facts.cached_latest.as_ref().and_then(|(latest, at)| {
        semver::Version::parse(latest)
            .ok()
            .map(|v| (latest.clone(), v, *at))
    });
    let (update_state, latest, checked_at) = match cached {
        Some((latest, parsed, at)) => {
            let newer = semver::Version::parse(i.version).is_ok_and(|current| parsed > current);
            (
                if newer { "available" } else { "current" },
                Some(latest),
                Some(at),
            )
        }
        None => ("unknown", None, None),
    };

    let project_root = scan
        .sources
        .first()
        .and_then(|s| s.path.parent())
        .map_or_else(|| i.cwd.to_owned(), Path::to_owned);

    let plugins = packs_report(&project_root, i.cwd, data.path.as_deref());
    let trust = trust_report(state.path.as_deref());

    let config = Config {
        ok: scan.sources.iter().all(|s| s.status == SourceStatus::Ok),
        sources: scan
            .sources
            .iter()
            .map(|s| DoctorConfigFile {
                layer: s.layer,
                file: path_for_log(&s.path, i.cwd),
                status: s.status,
                error: s.error.clone(),
            })
            .collect(),
    };

    let ok = config.ok
        && !matches!(plugins.lockfile.status, "invalid" | "unreadable")
        && plugins.items.iter().all(|p| p.state != "tampered")
        && trust.status != "unreadable";

    Report {
        schema_version: SCHEMA_VERSION,
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
        plugins,
        trust,
        directories: Directories {
            cache: directory(&cache, i.cwd, i.walk_limit),
            data: directory(&data, i.cwd, i.walk_limit),
            state: directory(&state, i.cwd, i.walk_limit),
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
                let hint = self.slopvac.upgrade_command.as_deref().map_or_else(
                    || format!(" See {}.", slopvac_update::install::RELEASES_URL),
                    |c| format!(" Upgrade with `{c}`."),
                );
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
        let lock = &self.plugins.lockfile;
        let detail = lock
            .error
            .as_deref()
            .map_or(String::new(), |e| format!(": {e}"));
        let _ = writeln!(out, "plugins: {} {}{detail}", lock.file, lock.status);
        for p in &self.plugins.items {
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
                upgrade_command: Some("brew upgrade slopvac/tap/slopvac".into()),
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
            dirs: &dirs::SharedDirs,
            walk_limit: WALK_LIMIT,
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
            ("3.0.0", "garbage", "unknown"),
        ] {
            let r = report(current, Some(cached));
            assert_eq!(r.update.state, state, "{current} vs {cached}");
        }
        assert_eq!(report("3.0.0", None).update.state, "unknown");
    }

    /// The cache entry of a report built over `dir` with the given walk limit.
    fn cache_entry(dir: &Path, walk_limit: usize) -> serde_json::Value {
        struct Dirs(PathBuf);
        impl super::Dirs for Dirs {
            fn cache(&self, _: &dyn Env) -> Located {
                Located::at(self.0.clone(), "test")
            }
            fn data(&self, _: &dyn Env) -> Located {
                Located::unresolved()
            }
            fn state(&self, _: &dyn Env) -> Located {
                Located::unresolved()
            }
        }
        let env = MapEnv::default();
        let report = build(&Inputs {
            env: &env,
            cwd: Path::new("/nonexistent-cwd"),
            offline_flag: false,
            explicit_config: None,
            version: "3.0.0",
            update: &Fixed(None),
            config: &stubs::LocalConfigSources,
            dirs: &Dirs(dir.to_owned()),
            walk_limit,
        });
        serde_json::from_str::<serde_json::Value>(&report.to_json()).unwrap()["directories"]
            ["cache"]
            .clone()
    }

    #[test]
    fn the_walk_limit_reaches_the_report_as_a_lower_bound_and_a_flag() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a", "b", "c", "d", "e"] {
            std::fs::write(dir.path().join(name), b"xy").unwrap();
        }
        // Five files of two bytes each: a limit of five sees them all.
        let whole = cache_entry(dir.path(), 5);
        assert_eq!(whole["bytes"], 10);
        assert_eq!(whole["truncated"], false);
        // A limit of three stops early: three files were counted, and the report says so.
        let cut = cache_entry(dir.path(), 3);
        assert_eq!(cut["bytes"], 6);
        assert_eq!(cut["truncated"], true);
    }

    #[test]
    fn directories_use_up_entries_but_add_no_bytes() {
        // x, x/y and x/y/f are three entries. Only f has a size.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("x/y")).unwrap();
        std::fs::write(dir.path().join("x/y/f"), b"123").unwrap();
        let exact = cache_entry(dir.path(), 3);
        assert_eq!(exact["bytes"], 3);
        assert_eq!(exact["truncated"], false);
        // At two, the walk stops before it reaches the file, so no bytes are counted.
        let cut = cache_entry(dir.path(), 2);
        assert_eq!(cut["bytes"], 0);
        assert_eq!(cut["truncated"], true);
    }

    #[test]
    fn lock_diagnostics_keep_only_known_keys_and_indexes() {
        let err = |pointer: &str| LockError {
            pointer: pointer.to_owned(),
            message: "echoes LEAK".to_owned(),
        };
        for (pointer, want) in [
            ("", "not a valid lockfile"),
            ("/plugins/12/digest", "invalid value at /plugins/12/digest"),
            ("/plugins/0/LEAK", "invalid value at /plugins/0/*"),
            (
                "/plugins/0/signature/key_id",
                "invalid value at /plugins/0/signature/key_id",
            ),
            ("/plugins/9999999", "invalid value at /plugins/*"),
        ] {
            assert_eq!(lock_diagnostic(&err(pointer)), want, "{pointer}");
        }
    }

    #[test]
    fn the_text_summary_names_the_upgrade_command_for_an_available_update() {
        let text = report("3.0.0", Some("3.1.0")).to_text();
        assert!(
            text.contains(
                "update: 3.1.0 is available. Upgrade with `brew upgrade slopvac/tap/slopvac`."
            ),
            "{text}"
        );
    }
}
