//! Config sources for `doctor`, from the real config layer.
//!
//! Every file of the stack is a source: the project file (or the `--config` file) and each file it
//! extends. A file that cannot be read or is refused is one source with a status and a fixed
//! message that holds no file content. `[network]` settings come from the resolved stack, as for
//! every other command.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use slopvac_config::{ConfigError, Discovery, Layer, LoadedConfig, NoPresets};
use slopvac_update::policy::policy_of;

use super::{ConfigScan, ConfigSources, RawSource, SourceStatus};

/// Uses `slopvac-config` discovery and loading.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayerConfigSources;

impl ConfigSources for LayerConfigSources {
    fn scan(&self, cwd: &Path, explicit: Option<&Path>) -> ConfigScan {
        let discovery = Discovery::new(&NoPresets);
        let loaded = match explicit {
            Some(given) => discovery.explicit_in(cwd, given).map(Some),
            None => discovery.nearest(cwd),
        };
        match loaded {
            Ok(Some(config)) => scan_loaded(&config, explicit.is_some()),
            Ok(None) => ConfigScan::default(),
            Err(err) => ConfigScan {
                sources: vec![failed(&err, cwd, explicit.is_some())],
                offline: false,
                update_check: None,
            },
        }
    }
}

fn scan_loaded(config: &LoadedConfig, explicit: bool) -> ConfigScan {
    let policy = policy_of(config);
    let sources = config
        .files
        .iter()
        .map(|file| RawSource {
            layer: match file.layer {
                Layer::Extends => "extends",
                _ if explicit => "explicit",
                _ => "project",
            },
            path: config.root.join(&file.display),
            status: SourceStatus::Ok,
            error: None,
        })
        .collect();
    ConfigScan {
        sources,
        offline: policy.offline,
        update_check: policy.update_check,
    }
}

fn failed(err: &ConfigError, cwd: &Path, explicit: bool) -> RawSource {
    let layer = if explicit { "explicit" } else { "project" };
    match err {
        ConfigError::Io { path, source } => {
            let (status, text) = match source.kind() {
                ErrorKind::NotFound => (SourceStatus::Missing, "file not found"),
                ErrorKind::InvalidInput => (SourceStatus::Unreadable, "not a regular file"),
                ErrorKind::FileTooLarge => (SourceStatus::Unreadable, "larger than 1048576 bytes"),
                ErrorKind::InvalidData => (SourceStatus::Unreadable, "not valid UTF-8"),
                ErrorKind::PermissionDenied => (SourceStatus::Unreadable, "permission denied"),
                _ => (SourceStatus::Unreadable, "cannot read the file"),
            };
            RawSource {
                layer,
                path: cwd.join(path),
                status,
                error: Some(text.to_owned()),
            }
        }
        ConfigError::Rejected(rejected) => {
            let (status, text) = if err.is_legacy() {
                (
                    SourceStatus::Legacy,
                    "2.x configuration: run `slopvac migrate`".to_owned(),
                )
            } else {
                let first = &rejected.diagnostics[0];
                let line = rejected.text[..first.span.start.min(rejected.text.len())]
                    .matches('\n')
                    .count()
                    + 1;
                let more = rejected.diagnostics.len() - 1;
                let extra = if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                };
                (
                    SourceStatus::Invalid,
                    format!("refused: {} at line {line}{extra}", first.code.as_str()),
                )
            };
            RawSource {
                layer,
                path: PathBuf::from(&rejected.file),
                status,
                error: Some(text),
            }
        }
    }
}
