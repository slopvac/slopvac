//! The cached update notice (`docs/contract/policies.md`, sections 1.2 and 1.3).
//!
//! Reading the notice never touches the network. A refresh runs only for the commands in
//! [`REFRESH_COMMANDS`], only when the cache is older than 24 hours, never in offline mode, and
//! never under `CI` or an opt-out. It also runs when stderr is not a terminal, because agents
//! read notices from machine output. A failed refresh is silent and leaves the cache alone.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fetch::{FetchError, Fetcher};
use crate::install::InstallMethod;

/// Commands that may start a refresh.
pub const REFRESH_COMMANDS: [&str; 4] = ["lint", "doctor", "prime", "self update"];
/// How long a cached answer stays fresh.
pub const CACHE_TTL_SECS: u64 = 24 * 60 * 60;
/// Longest a refresh may delay a command.
pub const REFRESH_DEADLINE: Duration = Duration::from_millis(1500);
const MAX_REGISTRY_BYTES: u64 = 4 * 1024 * 1024;
const GITHUB_LATEST: &str = "https://api.github.com/repos/srobroek/slopvac/releases/latest";
const PYPI_JSON: &str = "https://pypi.org/pypi/slopvac/json";

/// Reads an environment variable.
pub type VarFn<'a> = &'a dyn Fn(&str) -> Option<String>;

/// A machine-readable fact in the shape of `Notice` in `schemas/v3/lint-report.schema.json`.
///
/// The schema allows only `x-` keys beyond `code`, `level` and `message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    /// Always `update-available`.
    pub code: &'static str,
    /// Always `info`.
    pub level: &'static str,
    /// Human message. Not an interface.
    pub message: String,
    /// The running version.
    #[serde(rename = "x-current")]
    pub current: String,
    /// The newest release.
    #[serde(rename = "x-latest")]
    pub latest: String,
    /// The command that upgrades this install, when the channel is known.
    #[serde(rename = "x-upgrade-command", skip_serializing_if = "Option::is_none")]
    pub upgrade_command: Option<String>,
    /// Set when the release is a new major version.
    #[serde(rename = "x-migrate-hint", skip_serializing_if = "Option::is_none")]
    pub migrate_hint: Option<String>,
}

impl Notice {
    /// The one line `prime` prints.
    #[must_use]
    pub fn prime_line(&self) -> String {
        self.message.clone()
    }
}

/// Build the notice when `latest` is newer than `current`.
#[must_use]
pub fn notice_for(current: &str, latest: &str, method: InstallMethod) -> Option<Notice> {
    let cur = Version::parse(current).ok()?;
    let new = Version::parse(latest).ok()?;
    if new <= cur {
        return None;
    }
    let upgrade_command = method.upgrade_command().map(str::to_owned);
    let migrate_hint = (new.major > cur.major).then(|| {
        "This is a new major version. Run `slopvac migrate` to update your configuration."
            .to_owned()
    });
    let mut message = format!("slopvac {latest} is available (you have {current}).");
    match &upgrade_command {
        Some(cmd) => {
            message.push_str(" Upgrade with `");
            message.push_str(cmd);
            message.push_str("`.");
        }
        None => message.push_str(" See https://github.com/srobroek/slopvac/releases."),
    }
    if let Some(hint) = &migrate_hint {
        message.push(' ');
        message.push_str(hint);
    }
    Some(Notice {
        code: "update-available",
        level: "info",
        message,
        current: current.to_owned(),
        latest: latest.to_owned(),
        upgrade_command,
        migrate_hint,
    })
}

/// A cached registry answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Unix seconds of the check.
    pub checked_at: u64,
    /// The newest version the registry named.
    pub latest: String,
}

/// The on-disk cache file.
#[derive(Debug, Clone)]
pub struct Cache {
    path: PathBuf,
}

impl Cache {
    /// A cache stored at `path`.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// The default location: `SLOPVAC_CACHE_DIR`, else the platform cache directory.
    #[must_use]
    pub fn default_for(var: VarFn<'_>) -> Option<Self> {
        let non_empty = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let dir = non_empty("SLOPVAC_CACHE_DIR").or_else(|| {
            if cfg!(windows) {
                non_empty("LOCALAPPDATA").map(|d| d.join("slopvac"))
            } else {
                non_empty("XDG_CACHE_HOME")
                    .or_else(|| non_empty("HOME").map(|h| h.join(".cache")))
                    .map(|d| d.join("slopvac"))
            }
        })?;
        Some(Self::at(dir.join("update-check.json")))
    }

    /// The cached entry, or `None` when missing or unreadable.
    #[must_use]
    pub fn read(&self) -> Option<Entry> {
        serde_json::from_slice(&std::fs::read(&self.path).ok()?).ok()
    }

    /// Store an entry. The caller may ignore the error.
    pub fn write(&self, entry: &Entry) -> std::io::Result<()> {
        self.write_with(entry, |tmp| {
            tmp.persist(&self.path)
                .map(|_| ())
                .map_err(|e| (e.error, e.file))
        })
    }

    /// [`Cache::write`] with the rename step supplied, so a test can make it fail.
    ///
    /// This is the only place that retries the rename: `write` calls it, so a test that
    /// injects a failing step here fails when the retry is removed.
    fn write_with<F>(&self, entry: &Entry, persist: F) -> std::io::Result<()>
    where
        F: FnMut(tempfile::NamedTempFile) -> Result<(), (std::io::Error, tempfile::NamedTempFile)>,
    {
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)?;
        // A unique temp file per writer, renamed over the target: readers see the old or the new
        // entry, never a mix, and writers never share a file.
        let mut tmp = tempfile::Builder::new()
            .prefix(".update-check-")
            .tempfile_in(dir)?;
        std::io::Write::write_all(
            &mut tmp,
            &serde_json::to_vec(entry).map_err(std::io::Error::other)?,
        )?;
        persist_retrying(tmp, persist, PERSIST_RETRIES, PERSIST_PAUSE)
    }

    /// Path of the cache file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// How often a rename that Windows refuses for a moment is retried.
const PERSIST_RETRIES: u32 = 20;
const PERSIST_PAUSE: Duration = Duration::from_millis(10);

/// Run `persist` until it succeeds.
///
/// On Windows a rename over a file that another writer is replacing can fail with
/// `PermissionDenied` for a moment, so that error is retried up to `retries` times, with `pause`
/// between tries. Any other error, or the last `PermissionDenied`, is returned at once. The temp file
/// is deleted whenever this returns an error.
fn persist_retrying<F>(
    mut tmp: tempfile::NamedTempFile,
    mut persist: F,
    retries: u32,
    pause: Duration,
) -> std::io::Result<()>
where
    F: FnMut(tempfile::NamedTempFile) -> Result<(), (std::io::Error, tempfile::NamedTempFile)>,
{
    let mut attempt = 0;
    loop {
        match persist(tmp) {
            Ok(()) => return Ok(()),
            Err((e, file))
                if e.kind() == std::io::ErrorKind::PermissionDenied && attempt < retries =>
            {
                attempt += 1;
                std::thread::sleep(pause);
                tmp = file;
            }
            Err((e, _file)) => return Err(e),
        }
    }
}

fn set(var: VarFn<'_>, name: &str) -> bool {
    var(name).is_some_and(|v| !v.is_empty())
}

/// Whether CI or an opt-out silences the update check and its notice.
///
/// `CI` counts with any value, as in 2.x. `config` is `[network] update_check`.
#[must_use]
pub fn silenced(var: VarFn<'_>, config: Option<bool>) -> bool {
    var("CI").is_some()
        || [
            "NO_UPDATE_NOTIFIER",
            "SLOPVAC_NO_UPDATE_CHECK",
            "DO_NOT_TRACK",
        ]
        .iter()
        .any(|n| set(var, n))
        || config == Some(false)
}

/// Why asking a registry failed.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// Transport failure.
    #[error(transparent)]
    Fetch(#[from] FetchError),
    /// The answer is not what the registry documents.
    #[error("unexpected registry response")]
    Shape,
}

/// Ask the registry for the newest stable version.
///
/// `PyPI` answers for wheel installs. GitHub's latest release answers for every other channel; it
/// skips drafts and pre-releases.
pub fn latest_version(fetcher: &dyn Fetcher, pypi: bool) -> Result<String, RegistryError> {
    let url = if pypi { PYPI_JSON } else { GITHUB_LATEST };
    let body = fetcher.get(url, MAX_REGISTRY_BYTES)?;
    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| RegistryError::Shape)?;
    let raw = if pypi {
        json.pointer("/info/version")
    } else {
        json.get("tag_name")
    }
    .and_then(serde_json::Value::as_str)
    .ok_or(RegistryError::Shape)?;
    let version = raw.strip_prefix('v').unwrap_or(raw);
    Version::parse(version).map_err(|_| RegistryError::Shape)?;
    Ok(version.to_owned())
}

/// Everything the notice decision needs.
pub struct Context<'a> {
    /// Space-joined command path, such as `lint` or `self update`. `None` for `--version`.
    pub command: Option<&'a str>,
    /// Offline mode is on (sticky, already resolved).
    pub offline: bool,
    /// `[network] update_check` from configuration.
    pub config_update_check: Option<bool>,
    /// Environment access.
    pub var: VarFn<'a>,
    /// Unix seconds now.
    pub now: u64,
    /// The running version.
    pub current: &'a str,
    /// How this binary was installed.
    pub method: InstallMethod,
}

/// Refresh the cache when stale, then return the notice for `ctx.current`, if any.
///
/// `fetcher` is `None` to read the cache only. The refresh waits at most `deadline`.
#[must_use]
pub fn notice(
    ctx: &Context<'_>,
    cache: &Cache,
    fetcher: Option<Arc<dyn Fetcher + Send + Sync>>,
    deadline: Duration,
) -> Option<Notice> {
    if silenced(ctx.var, ctx.config_update_check) {
        return None;
    }
    let may_refresh = !ctx.offline && ctx.command.is_some_and(|c| REFRESH_COMMANDS.contains(&c));
    let mut entry = cache.read();
    let stale = entry
        .as_ref()
        .is_none_or(|e| ctx.now.saturating_sub(e.checked_at) >= CACHE_TTL_SECS);
    if stale
        && may_refresh
        && let Some(fetcher) = fetcher
        && let Some(fresh) = refresh(fetcher, ctx.method.uses_pypi(), ctx.now, deadline)
    {
        let _ = cache.write(&fresh);
        entry = Some(fresh);
    }
    notice_for(ctx.current, &entry?.latest, ctx.method)
}

fn refresh(
    fetcher: Arc<dyn Fetcher + Send + Sync>,
    pypi: bool,
    now: u64,
    deadline: Duration,
) -> Option<Entry> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send(latest_version(fetcher.as_ref(), pypi));
    });
    let latest = rx.recv_timeout(deadline).ok()?.ok()?;
    Some(Entry {
        checked_at: now,
        latest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Canned {
        body: &'static str,
        calls: AtomicUsize,
        urls: std::sync::Mutex<Vec<String>>,
    }

    impl Canned {
        fn new(body: &'static str) -> Arc<Self> {
            Arc::new(Self {
                body,
                calls: AtomicUsize::new(0),
                urls: std::sync::Mutex::default(),
            })
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Fetcher for Canned {
        fn get(&self, url: &str, _limit: u64) -> Result<Vec<u8>, FetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.urls.lock().unwrap().push(url.to_owned());
            Ok(self.body.as_bytes().to_vec())
        }
    }

    const GH: &str = r#"{"tag_name":"v3.1.0"}"#;
    const NO_ENV: VarFn<'static> = &|_| None;
    const T0: u64 = 1_000_000;

    fn ctx<'a>(command: Option<&'a str>, var: VarFn<'a>, now: u64) -> Context<'a> {
        Context {
            command,
            offline: false,
            config_update_check: None,
            var,
            now,
            current: "3.0.0",
            method: InstallMethod::Homebrew,
        }
    }

    fn cache() -> (tempfile::TempDir, Cache) {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("c/update-check.json"));
        (dir, cache)
    }

    fn entry(checked_at: u64, latest: &str) -> Entry {
        Entry {
            checked_at,
            latest: latest.into(),
        }
    }

    fn run(ctx: &Context<'_>, cache: &Cache, f: &Arc<Canned>) -> Option<Notice> {
        notice(ctx, cache, Some(f.clone()), Duration::from_secs(5))
    }

    #[test]
    fn stale_cache_refreshes_without_a_terminal_and_names_the_upgrade() {
        let (_d, cache) = cache();
        let f = Canned::new(GH);
        let n = run(&ctx(Some("lint"), NO_ENV, T0), &cache, &f).expect("notice");
        assert_eq!(n.latest, "3.1.0");
        assert_eq!(
            n.upgrade_command.as_deref(),
            Some("brew upgrade srobroek/tap/slopvac")
        );
        assert_eq!(n.migrate_hint, None);
        assert_eq!(cache.read().unwrap(), entry(T0, "3.1.0"));
    }

    #[test]
    fn fresh_cache_does_not_hit_the_network() {
        let (_d, cache) = cache();
        cache.write(&entry(T0, "3.2.0")).unwrap();
        let f = Canned::new(GH);
        let c = ctx(Some("lint"), NO_ENV, T0 + CACHE_TTL_SECS - 1);
        assert_eq!(run(&c, &cache, &f).unwrap().latest, "3.2.0");
        assert_eq!(f.calls(), 0);
    }

    #[test]
    fn cache_older_than_a_day_refreshes() {
        let (_d, cache) = cache();
        cache.write(&entry(T0, "3.2.0")).unwrap();
        let f = Canned::new(GH);
        let c = ctx(Some("doctor"), NO_ENV, T0 + CACHE_TTL_SECS);
        assert_eq!(run(&c, &cache, &f).unwrap().latest, "3.1.0");
        assert_eq!(f.calls(), 1);
    }

    #[test]
    fn offline_never_fetches_but_shows_the_cached_notice() {
        let (_d, cache) = cache();
        let f = Canned::new(GH);
        let mut c = ctx(Some("lint"), NO_ENV, T0);
        c.offline = true;
        assert_eq!(run(&c, &cache, &f), None);
        cache.write(&entry(1, "3.5.0")).unwrap();
        assert_eq!(run(&c, &cache, &f).unwrap().latest, "3.5.0");
        assert_eq!(f.calls(), 0);
    }

    #[test]
    fn ci_and_opt_outs_silence_check_and_notice() {
        for name in [
            "CI",
            "NO_UPDATE_NOTIFIER",
            "SLOPVAC_NO_UPDATE_CHECK",
            "DO_NOT_TRACK",
        ] {
            let (_d, cache) = cache();
            cache.write(&entry(1, "9.0.0")).unwrap();
            let f = Canned::new(GH);
            let var = move |k: &str| (k == name).then(|| "1".to_owned());
            assert_eq!(
                run(&ctx(Some("lint"), &var, T0), &cache, &f),
                None,
                "{name}"
            );
            assert_eq!(f.calls(), 0, "{name}");
        }
    }

    #[test]
    fn empty_opt_out_variable_does_not_silence() {
        let (_d, cache) = cache();
        let f = Canned::new(GH);
        let var = |k: &str| (k == "DO_NOT_TRACK").then(String::new);
        assert!(run(&ctx(Some("lint"), &var, T0), &cache, &f).is_some());
    }

    #[test]
    fn config_update_check_false_silences() {
        let (_d, cache) = cache();
        let f = Canned::new(GH);
        let mut c = ctx(Some("lint"), NO_ENV, T0);
        c.config_update_check = Some(false);
        assert_eq!(run(&c, &cache, &f), None);
        assert_eq!(f.calls(), 0);
    }

    #[test]
    fn only_listed_commands_start_a_refresh() {
        for (command, expect) in [
            (Some("lint"), 1),
            (Some("doctor"), 1),
            (Some("prime"), 1),
            (Some("self update"), 1),
            (Some("rules"), 0),
            (Some("completions"), 0),
            (None, 0),
        ] {
            let (_d, cache) = cache();
            let f = Canned::new(GH);
            let _ = run(&ctx(command, NO_ENV, T0), &cache, &f);
            assert_eq!(f.calls(), expect, "{command:?}");
        }
    }

    #[test]
    fn wheel_installs_ask_pypi() {
        let (_d, cache) = cache();
        let f = Canned::new(r#"{"info":{"version":"3.4.0"}}"#);
        let mut c = ctx(Some("lint"), NO_ENV, T0);
        c.method = InstallMethod::Wheel(crate::install::WheelTool::Uv);
        assert_eq!(run(&c, &cache, &f).unwrap().latest, "3.4.0");
        assert_eq!(
            f.urls.lock().unwrap()[0],
            "https://pypi.org/pypi/slopvac/json"
        );
    }

    #[test]
    fn failed_refresh_is_silent_and_keeps_the_cache() {
        struct Down;
        impl Fetcher for Down {
            fn get(&self, _: &str, _: u64) -> Result<Vec<u8>, FetchError> {
                Err(FetchError::Network("down".into()))
            }
        }
        let (_d, cache) = cache();
        cache.write(&entry(1, "3.1.0")).unwrap();
        let c = ctx(Some("lint"), NO_ENV, T0);
        let n = notice(&c, &cache, Some(Arc::new(Down)), Duration::from_secs(5));
        assert_eq!(n.unwrap().latest, "3.1.0");
        assert_eq!(cache.read().unwrap().checked_at, 1);
    }

    #[test]
    fn slow_refresh_gives_up_at_the_deadline() {
        struct Slow;
        impl Fetcher for Slow {
            fn get(&self, _: &str, _: u64) -> Result<Vec<u8>, FetchError> {
                std::thread::sleep(Duration::from_secs(2));
                Ok(GH.as_bytes().to_vec())
            }
        }
        let (_d, cache) = cache();
        let c = ctx(Some("lint"), NO_ENV, T0);
        let n = notice(&c, &cache, Some(Arc::new(Slow)), Duration::from_millis(50));
        assert_eq!(n, None);
    }

    #[test]
    fn notice_for_compares_versions() {
        let m = InstallMethod::Unknown;
        assert!(notice_for("3.0.0", "3.0.0", m).is_none());
        assert!(notice_for("3.1.0", "3.0.9", m).is_none());
        assert!(notice_for("3.1.0-rc.1", "3.1.0", m).is_some());
        assert!(notice_for("garbage", "3.1.0", m).is_none());
    }

    #[test]
    fn major_bump_carries_the_migrate_hint() {
        let n = notice_for("2.9.0", "3.0.0", InstallMethod::Installer).unwrap();
        assert!(
            n.migrate_hint
                .as_deref()
                .unwrap()
                .contains("slopvac migrate")
        );
        assert!(n.message.contains("slopvac migrate"));
        let minor = notice_for("3.0.0", "3.1.0", InstallMethod::Installer).unwrap();
        assert_eq!(minor.migrate_hint, None);
    }

    #[test]
    fn unknown_channel_points_at_releases() {
        let n = notice_for("3.0.0", "3.1.0", InstallMethod::Unknown).unwrap();
        assert_eq!(n.upgrade_command, None);
        assert!(n.message.contains("github.com/srobroek/slopvac/releases"));
    }

    #[test]
    fn notice_serialises_to_the_schema_shape() {
        let n = notice_for("2.0.0", "3.0.0", InstallMethod::Installer).unwrap();
        let v = serde_json::to_value(&n).unwrap();
        let keys: std::collections::BTreeSet<String> =
            v.as_object().unwrap().keys().cloned().collect();
        let want: std::collections::BTreeSet<String> = [
            "code",
            "level",
            "message",
            "x-current",
            "x-latest",
            "x-upgrade-command",
            "x-migrate-hint",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(keys, want);
        assert_eq!(v["code"], "update-available");
        assert_eq!(v["level"], "info");
    }

    fn temp_in(dir: &Path) -> tempfile::NamedTempFile {
        tempfile::Builder::new()
            .prefix(".update-check-")
            .tempfile_in(dir)
            .unwrap()
    }

    fn denied() -> std::io::Error {
        std::io::Error::from(std::io::ErrorKind::PermissionDenied)
    }

    fn dir_is_empty(dir: &Path) -> bool {
        std::fs::read_dir(dir).unwrap().next().is_none()
    }

    #[test]
    fn a_transient_permission_denied_is_retried_until_it_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let mut calls = 0;
        let result = persist_retrying(
            temp_in(dir.path()),
            |tmp| {
                calls += 1;
                if calls < 4 {
                    Err((denied(), tmp))
                } else {
                    Ok(())
                }
            },
            20,
            Duration::ZERO,
        );
        assert!(result.is_ok());
        assert_eq!(calls, 4);
    }

    #[test]
    fn persistent_permission_denied_returns_the_last_error_after_the_retry_budget() {
        let dir = tempfile::tempdir().unwrap();
        let mut calls = 0;
        let err = persist_retrying(
            temp_in(dir.path()),
            |tmp| {
                calls += 1;
                Err((denied(), tmp))
            },
            3,
            Duration::ZERO,
        )
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(calls, 4, "one try plus three retries");
        assert!(dir_is_empty(dir.path()), "the temp file is cleaned up");
    }

    #[test]
    fn other_errors_are_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        let mut calls = 0;
        let err = persist_retrying(
            temp_in(dir.path()),
            |tmp| {
                calls += 1;
                Err((std::io::Error::from(std::io::ErrorKind::NotFound), tmp))
            },
            20,
            Duration::ZERO,
        )
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(calls, 1);
        assert!(dir_is_empty(dir.path()), "the temp file is cleaned up");
    }

    /// Fail the rename `failures` times with `PermissionDenied`, then do the real rename.
    fn flaky_persist(
        target: &Path,
        failures: u32,
        calls: &std::cell::Cell<u32>,
    ) -> impl FnMut(tempfile::NamedTempFile) -> Result<(), (std::io::Error, tempfile::NamedTempFile)>
    {
        let target = target.to_path_buf();
        move |tmp| {
            calls.set(calls.get() + 1);
            if calls.get() <= failures {
                Err((denied(), tmp))
            } else {
                tmp.persist(&target)
                    .map(|_| ())
                    .map_err(|e| (e.error, e.file))
            }
        }
    }

    #[test]
    fn cache_write_retries_a_transient_permission_denied_and_stores_the_entry() {
        let (_dir, cache) = cache();
        let calls = std::cell::Cell::new(0);
        cache
            .write_with(&entry(7, "9.9.9"), flaky_persist(cache.path(), 3, &calls))
            .expect("the write succeeds after the retries");
        assert_eq!(calls.get(), 4, "three refusals and one success");
        assert_eq!(cache.read().expect("stored").latest, "9.9.9");
    }

    #[test]
    fn cache_write_gives_up_after_its_retry_budget_and_leaves_no_temp_file() {
        let (dir, cache) = cache();
        let calls = std::cell::Cell::new(0);
        let err = cache
            .write_with(
                &entry(7, "9.9.9"),
                flaky_persist(cache.path(), u32::MAX, &calls),
            )
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(calls.get(), PERSIST_RETRIES + 1, "one try plus the retries");
        assert!(cache.read().is_none());
        assert!(
            dir_is_empty(&dir.path().join("c")),
            "the temp file is cleaned up"
        );
    }

    #[test]
    fn overlapping_writers_never_corrupt_or_leave_temp_files() {
        let (dir, cache) = cache();
        let cache = Arc::new(cache);
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let cache = cache.clone();
                std::thread::spawn(move || {
                    for round in 0..10 {
                        cache
                            .write(&entry(i * 100 + round, &format!("3.{i}.{round}")))
                            .expect("write");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let last = cache.read().expect("parsable after the race");
        assert!(last.latest.starts_with("3."));
        let files: Vec<_> = std::fs::read_dir(dir.path().join("c"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, ["update-check.json"]);
    }

    #[test]
    fn default_cache_path_follows_the_environment() {
        let var = |k: &str| match k {
            "SLOPVAC_CACHE_DIR" => Some("/c".to_owned()),
            "HOME" => Some("/h".to_owned()),
            _ => None,
        };
        assert_eq!(
            Cache::default_for(&var).unwrap().path(),
            Path::new("/c/update-check.json")
        );
    }
}
