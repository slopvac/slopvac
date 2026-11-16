//! The `self` subtree and its entry points.
//!
//! The CLI mounts [`command`] and calls [`run`] for `slopvac self ...`.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{ArgMatches, Command};

use crate::fetch::{Fetcher, UreqFetcher};
use crate::install::{self, InstallMethod};
use crate::notice::{self, Cache, Context, Notice};
use crate::pipeline::{self, DOWNLOAD_BASE, UpdateError};
use crate::verify::{ArtifactVerifier, ReleaseVerifier};

/// A boxed environment reader.
pub type OwnedVar = Box<dyn Fn(&str) -> Option<String>>;

/// Environment variables this crate reads.
pub const ENV_VARS: [&str; 11] = [
    "CI",
    "NO_UPDATE_NOTIFIER",
    "SLOPVAC_NO_UPDATE_CHECK",
    "DO_NOT_TRACK",
    "AXOUPDATER_CONFIG_PATH",
    "XDG_CONFIG_HOME",
    "LOCALAPPDATA",
    "HOME",
    "CARGO_HOME",
    "SLOPVAC_CACHE_DIR",
    "XDG_CACHE_HOME",
];

/// A command failure: a stable kebab-case code and a message. The CLI exits 2 for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Stable code.
    pub code: &'static str,
    /// Human message. Not an interface.
    pub message: String,
}

impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Everything `self update` and the notice read from the outside world.
pub struct Runtime {
    /// Offline mode is on.
    pub offline: bool,
    /// `[network] update_check`.
    pub config_update_check: Option<bool>,
    /// The running version.
    pub current: String,
    /// Resolved path of the running executable.
    pub exe: PathBuf,
    /// Environment access.
    pub var: OwnedVar,
    /// Unix seconds now.
    pub now: u64,
    /// HTTP.
    pub fetcher: Arc<dyn Fetcher + Send + Sync>,
    /// Signature check for downloads.
    pub verifier: Box<dyn ArtifactVerifier>,
    /// Where release assets live.
    pub download_base: String,
    /// Target triple, `None` on an unsupported platform.
    pub target: Option<&'static str>,
    /// Cache file, `None` when no cache directory exists.
    pub cache: Option<Cache>,
}

impl Runtime {
    /// The real process: current executable, real environment and clock, real network.
    #[must_use]
    pub fn system(offline: bool, current: &str, config_update_check: Option<bool>) -> Self {
        Self::with_var(
            offline,
            current,
            config_update_check,
            Box::new(|k| std::env::var(k).ok()),
        )
    }

    /// Like [`Runtime::system`], reading variables through `var`. [`ENV_VARS`] lists the names read.
    #[must_use]
    pub fn with_var(
        offline: bool,
        current: &str,
        config_update_check: Option<bool>,
        var: OwnedVar,
    ) -> Self {
        let exe = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .unwrap_or_default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let cache = Cache::default_for(&*var);
        Self {
            offline,
            config_update_check,
            current: current.to_owned(),
            exe,
            var,
            now,
            fetcher: Arc::new(UreqFetcher::new(std::time::Duration::from_secs(30))),
            verifier: Box::new(ReleaseVerifier::default()),
            download_base: DOWNLOAD_BASE.to_owned(),
            target: pipeline::target_triple(),
            cache,
        }
    }

    /// How this binary was installed.
    #[must_use]
    pub fn method(&self) -> InstallMethod {
        install::detect(&self.exe, &*self.var)
    }

    /// The update notice for `command` (`lint`, `self update`, ...), or `None` for `--version`.
    ///
    /// Refreshes the 24-hour cache when the command may. Never fails: errors mean no notice.
    #[must_use]
    pub fn notice(&self, command: Option<&str>) -> Option<Notice> {
        let ctx = Context {
            command,
            offline: self.offline,
            config_update_check: self.config_update_check,
            var: &*self.var,
            now: self.now,
            current: &self.current,
            method: self.method(),
        };
        notice::notice(
            &ctx,
            self.cache.as_ref()?,
            Some(self.fetcher.clone()),
            notice::REFRESH_DEADLINE,
        )
    }
}

/// The `self` subtree.
#[must_use]
pub fn command() -> Command {
    Command::new("self")
        .about("Manage this installation.")
        .subcommand_required(true)
        .subcommand(
            Command::new("update")
                .about("Update slopvac to the latest release.")
                .long_about(
                    "Update slopvac to the latest release. Works for installs made by the slopvac \
                 installer. For any other install it prints the command that upgrades it. The \
                 download must carry a valid minisign signature, or nothing is replaced.",
                ),
        )
}

/// Run a `self` subcommand. `matches` is the match set of [`command`]. Returns the text for stdout.
pub fn run(matches: &ArgMatches, rt: &Runtime) -> Result<String, Failure> {
    match matches.subcommand() {
        Some(("update", _)) => self_update(rt),
        _ => Err(Failure::new("usage", "unknown `self` subcommand")),
    }
}

fn self_update(rt: &Runtime) -> Result<String, Failure> {
    let method = rt.method();
    if method != InstallMethod::Installer {
        let advice = method.upgrade_command().map_or_else(
            || "download a release from https://github.com/srobroek/slopvac/releases".to_owned(),
            |c| format!("run `{c}`"),
        );
        return Err(Failure::new(
            "self-update-unmanaged",
            format!(
                "this install ({}) is not managed by the slopvac installer; {advice}",
                method.id()
            ),
        ));
    }
    if rt.offline {
        return Err(Failure::new(
            "offline-network-needed",
            "offline: self update needs the network",
        ));
    }
    let latest = notice::latest_version(&*rt.fetcher, false)
        .map_err(|e| Failure::new("update-check-failed", e.to_string()))?;
    if notice::notice_for(&rt.current, &latest, method).is_none() {
        return Ok(format!("slopvac {} is up to date.\n", rt.current));
    }
    let target = rt.target.ok_or_else(|| {
        Failure::new(
            "self-update-failed",
            UpdateError::UnsupportedPlatform.to_string(),
        )
    })?;
    pipeline::update(
        &*rt.fetcher,
        &*rt.verifier,
        &rt.download_base,
        &rt.current,
        &latest,
        target,
        &rt.exe,
    )
    .map_err(|e| {
        let code = match e {
            UpdateError::Verify(_) => "update-verification-failed",
            _ => "self-update-failed",
        };
        Failure::new(code, e.to_string())
    })?;
    if let Some(cache) = &rt.cache {
        let _ = cache.write(&notice::Entry {
            checked_at: rt.now,
            latest: latest.clone(),
        });
    }
    Ok(format!("Updated slopvac {} to {latest}.\n", rt.current))
}

/// The `slopvac --version --json` document.
#[must_use]
pub fn version_json(rt: &Runtime) -> String {
    let notices: Vec<Notice> = rt.notice(None).into_iter().collect();
    let doc = serde_json::json!({
        "name": "slopvac",
        "version": rt.current,
        "install_method": rt.method().id(),
        "notices": notices,
    });
    let mut text = doc.to_string();
    text.push('\n');
    text
}
