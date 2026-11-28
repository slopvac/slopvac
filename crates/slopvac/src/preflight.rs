//! The command preflight: one resolution of the configuration for the working directory,
//! before a command runs (docs/contract/config.md, section 2.2).
//!
//! The resolution is the `--config` file of the command when it names one, or else the nearest
//! config of the working directory, with every file it extends. The network settings
//! `[network] offline` and `update_check` come from it, so offline stays sticky across the whole
//! `extends` chain. `lint` still resolves the nearest config of each input itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::ArgMatches;
use slopvac_config::{ConfigError, Discovery, LoadedConfig};
use slopvac_styles::{BuiltinSource, StylePresets};

use crate::errors::CliError;

/// How an invocation treats the preflight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Help, plain version text, and the commands that create, check or convert a config,
    /// read no project config.
    Skip,
    /// `report` reads the config for its offline setting, and goes on when it does not
    /// resolve, so that a user with a broken config can still file a bug.
    Tolerant,
    /// `doctor` reports a config that does not resolve and does not stop on it.
    Report,
    /// Every other command stops on a config that does not resolve.
    Strict,
}

/// The commands that skip the preflight.
const SKIPPED: [&str; 7] = [
    "completions",
    "man",
    "cli-reference",
    "init",
    "validate",
    "migrate",
    "lsp",
];

impl Mode {
    /// The mode of a parsed invocation. `slopvac` with no command prints help.
    pub(crate) fn of(matches: &ArgMatches) -> Self {
        match matches.subcommand_name() {
            None => Self::Skip,
            Some(name) if SKIPPED.contains(&name) => Self::Skip,
            Some("report") => Self::Tolerant,
            Some("doctor") => Self::Report,
            Some(_) => Self::Strict,
        }
    }
}

/// The `--config` value of the command, when it takes one and it is given.
pub(crate) fn explicit(matches: &ArgMatches) -> Option<PathBuf> {
    let (_, m) = matches.subcommand()?;
    m.try_get_one::<PathBuf>("config").ok().flatten().cloned()
}

/// The network settings of a resolved config.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Network {
    /// `[network] offline` is on in some layer.
    pub(crate) offline: bool,
    /// The resolved `[network] update_check`.
    pub(crate) update_check: Option<bool>,
}

impl Network {
    /// The settings that `config` resolves to. Offline is sticky: once any layer turns it on,
    /// no later layer turns it off.
    pub(crate) fn of(config: &LoadedConfig) -> Self {
        let resolved = crate::lint_settings::resolve(config, Path::new("."));
        Self {
            offline: resolved.is_offline(),
            update_check: resolved
                .get(&["network", "update_check"])
                .and_then(serde_json::Value::as_bool),
        }
    }
}

/// What the preflight resolved.
pub(crate) struct Preflight {
    mode: Mode,
    explicit: Option<PathBuf>,
    /// `None` when the invocation skips the preflight.
    outcome: Option<Result<Option<Arc<LoadedConfig>>, ConfigError>>,
    network: Network,
}

impl Preflight {
    /// Resolve the configuration of `cwd` for an invocation in `mode`.
    pub(crate) fn run(mode: Mode, cwd: &Path, explicit: Option<PathBuf>) -> Self {
        if mode == Mode::Skip {
            return Self {
                mode,
                explicit,
                outcome: None,
                network: Network::default(),
            };
        }
        let presets = StylePresets(BuiltinSource);
        let discovery = Discovery::new(&presets);
        let outcome = match &explicit {
            Some(given) => discovery.explicit_in(cwd, given).map(Some),
            None => discovery.nearest(cwd),
        };
        let network = match &outcome {
            Ok(Some(config)) => Network::of(config),
            Ok(None) => Network::default(),
            // No update check starts while the configuration does not resolve.
            Err(_) => Network {
                offline: false,
                update_check: Some(false),
            },
        };
        Self {
            mode,
            explicit,
            outcome: Some(outcome),
            network,
        }
    }

    /// The network settings to run with.
    pub(crate) fn network(&self) -> Network {
        self.network
    }

    /// The `--config` value the resolution used.
    pub(crate) fn explicit(&self) -> Option<&Path> {
        self.explicit.as_deref()
    }

    /// The resolution, or `None` when the invocation skipped it.
    pub(crate) fn outcome(&self) -> Option<&Result<Option<Arc<LoadedConfig>>, ConfigError>> {
        self.outcome.as_ref()
    }

    /// The failure that stops the command: the line `lint` prints for it. `None` when the
    /// configuration resolved, or when the invocation does not stop on a failure.
    pub(crate) fn refusal(&self, cwd: &Path) -> Option<CliError> {
        if self.mode != Mode::Strict {
            return None;
        }
        let Some(Err(err)) = &self.outcome else {
            return None;
        };
        if let Some(given) = &self.explicit
            && !cwd.join(given).is_file()
        {
            return Some(crate::engine::config_not_found(given, cwd));
        }
        Some(crate::engine::config_error(err, cwd))
    }
}
