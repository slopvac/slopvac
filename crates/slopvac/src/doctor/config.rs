//! The `config` section of `slopvac doctor`: every file the command preflight resolved, each
//! with a status and a fixed message (docs/contract/doctor.md, section 2.3).
//!
//! The loader checks each file in full before it reads the files that file extends, so every
//! file it reached before a failure is `ok`, and the list ends at the failure. The loader's own
//! diagnostics can quote keys and values, so each failure class maps to a fixed text here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use slopvac_config::{Code, ConfigBudget, ConfigError, Diagnostic, LoadedConfig, Rejected};
use slopvac_messages::{Catalog, Entry};

use super::{ConfigScan, RawSource, SourceStatus};

/// The `error` text of a config source that is not `ok`. None holds file content.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
enum SourceError {
    /// The file does not exist.
    #[code("doctor-config-not-found")]
    #[text("file not found")]
    NotFound,
    /// The file is over the config size cap.
    #[code("doctor-config-too-large")]
    #[text("larger than {cap} bytes")]
    TooLarge {
        /// The cap, in bytes.
        #[count]
        cap: u64,
    },
    /// The file is not valid UTF-8.
    #[code("doctor-config-not-utf8")]
    #[text("not valid UTF-8")]
    NotUtf8,
    /// Any other read failure.
    #[code("doctor-config-unreadable")]
    #[text("cannot read the file")]
    Unreadable,
    /// A 2.x file.
    #[code("doctor-config-legacy")]
    #[text("2.x configuration: run `slopvac migrate`")]
    Legacy,
    /// The file is not valid TOML.
    #[code("doctor-config-not-toml")]
    #[text("not valid TOML (line {line})")]
    NotToml {
        /// The one-based line of the first problem.
        line: String,
    },
    /// A `schema_version` newer than this build reads.
    #[code("doctor-config-unsupported-version")]
    #[text("unsupported schema_version")]
    UnsupportedVersion,
    /// A key the schema does not define.
    #[code("doctor-config-unknown-key")]
    #[text("unknown key (line {line})")]
    UnknownKey {
        /// The one-based line of the first problem.
        line: String,
    },
    /// Any other validation failure.
    #[code("doctor-config-invalid")]
    #[text("invalid configuration (line {line})")]
    Invalid {
        /// The one-based line of the first problem.
        line: String,
    },
    /// The `extends` entries form a loop.
    #[code("doctor-config-extends-loop")]
    #[text("the `extends` entries form a loop (line {line})")]
    ExtendsLoop {
        /// The one-based line of the entry that closes the loop.
        line: String,
    },
    /// An `extends` budget is passed.
    #[code("doctor-config-extends-budget")]
    #[text("the `extends` chain is over its budget (line {line})")]
    ExtendsBudget {
        /// The one-based line of the entry that passes the budget.
        line: String,
    },
    /// An `extends` entry names a plugin config that is not available.
    #[code("doctor-config-extends-no-plugin")]
    #[text("an `extends` entry names a plugin config that is not available (line {line})")]
    ExtendsNoPlugin {
        /// The one-based line of the entry.
        line: String,
    },
}

/// The entries of the doctor config messages, for the message registry.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    SourceError::entries()
}

/// The config sources of a doctor run, from the preflight's resolution of `cwd`. `explicit` is
/// the `--config` value, relative to `cwd`.
#[must_use]
pub fn scan(
    cwd: &Path,
    explicit: Option<&Path>,
    outcome: &Result<Option<Arc<LoadedConfig>>, ConfigError>,
) -> ConfigScan {
    let top = if explicit.is_some() {
        "explicit"
    } else {
        "project"
    };
    let layer = |index: usize| if index == 0 { top } else { "extends" };
    match outcome {
        Ok(None) => ConfigScan::default(),
        Ok(Some(config)) => {
            let network = crate::preflight::Network::of(config);
            ConfigScan {
                sources: config
                    .dependencies()
                    .iter()
                    .enumerate()
                    .map(|(i, path)| ok(layer(i), path))
                    .collect(),
                offline: network.offline,
                update_check: network.update_check,
                unresolved: false,
            }
        }
        Err(err) => ConfigScan {
            sources: failed(cwd, explicit, err, &layer),
            offline: false,
            update_check: None,
            unresolved: true,
        },
    }
}

fn ok(layer: &'static str, path: &Path) -> RawSource {
    RawSource {
        layer,
        path: path.to_owned(),
        status: SourceStatus::Ok,
        error: None,
    }
}

fn source(
    layer: &'static str,
    path: &Path,
    status: SourceStatus,
    error: &SourceError,
) -> RawSource {
    RawSource {
        layer,
        path: path.to_owned(),
        status,
        error: Some(error.to_string()),
    }
}

fn too_large() -> SourceError {
    SourceError::TooLarge {
        cap: slopvac_config::MAX_CONFIG_BYTES,
    }
}

/// The sources up to and including the one that failed.
fn failed(
    cwd: &Path,
    explicit: Option<&Path>,
    err: &ConfigError,
    layer: &dyn Fn(usize) -> &'static str,
) -> Vec<RawSource> {
    // A read failure of the top file names that file, or nothing when `--config` names a path
    // that does not resolve.
    let top = || {
        err.source_file()
            .map(Path::to_path_buf)
            .or_else(|| explicit.map(|given| cwd.join(given)))
    };
    match err {
        ConfigError::Io { source: cause, .. } => {
            let Some(path) = top() else {
                return Vec::new();
            };
            let (status, error) = match cause.kind() {
                std::io::ErrorKind::NotFound => (SourceStatus::Missing, SourceError::NotFound),
                std::io::ErrorKind::InvalidData => (SourceStatus::Unreadable, SourceError::NotUtf8),
                _ => (SourceStatus::Unreadable, SourceError::Unreadable),
            };
            vec![source(layer(0), &path, status, &error)]
        }
        ConfigError::TooLarge { .. } => top().map_or_else(Vec::new, |path| {
            vec![source(
                layer(0),
                &path,
                SourceStatus::Unreadable,
                &too_large(),
            )]
        }),
        ConfigError::Rejected(rejected) => refused(rejected, layer),
    }
}

/// The sources of a refused load: the files it reached, ending at the one that failed.
fn refused(rejected: &Rejected, layer: &dyn Fn(usize) -> &'static str) -> Vec<RawSource> {
    let reads = &rejected.reads;
    let Some(first) = rejected.diagnostics.first() else {
        return Vec::new();
    };
    // An extended file that could not be read takes the failure. The loader records it last,
    // before it reads it, and the file that names it stays `ok`.
    if let Some(target) = unread_target(rejected, first) {
        let mut sources: Vec<RawSource> = reads[..reads.len() - 1]
            .iter()
            .enumerate()
            .map(|(i, path)| ok(layer(i), path))
            .collect();
        let (status, error) = if first.code == Code::TooLarge {
            (SourceStatus::Unreadable, too_large())
        } else if std::fs::metadata(target).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            (SourceStatus::Missing, SourceError::NotFound)
        } else {
            (SourceStatus::Unreadable, SourceError::Unreadable)
        };
        sources.push(source(layer(reads.len() - 1), target, status, &error));
        return sources;
    }
    let Some(path) = rejected.path.as_deref() else {
        return Vec::new();
    };
    let failing = reads.iter().position(|p| p == path);
    let mut sources: Vec<RawSource> = failing
        .map_or(&[][..], |at| &reads[..at])
        .iter()
        .enumerate()
        .map(|(i, p)| ok(layer(i), p))
        .collect();
    let (status, error) = classify(rejected, first);
    sources.push(source(layer(sources.len()), path, status, &error));
    sources
}

/// The extended file whose read failed, when that is the refusal: the last file the load
/// reached, named by a path entry and not by a plugin config.
fn unread_target<'a>(rejected: &'a Rejected, first: &Diagnostic) -> Option<&'a PathBuf> {
    if first.path != ["extends"] || !matches!(first.code, Code::TooLarge | Code::ExtendsMissing) {
        return None;
    }
    let entry = rejected.text.get(first.span.clone())?;
    let name = toml::from_str::<toml::Table>(&format!("v = {entry}"))
        .ok()?
        .get("v")?
        .as_str()?
        .to_owned();
    // The loader's rule for a plugin config: `<plugin>:<name>`, never an absolute path.
    if name.contains(':') && !Path::new(&name).is_absolute() {
        return None;
    }
    let target = rejected.reads.last()?;
    (rejected.path.as_ref() != Some(target)).then_some(target)
}

/// The status and text of the file that holds the first problem.
fn classify(rejected: &Rejected, first: &Diagnostic) -> (SourceStatus, SourceError) {
    let line = first.location(&rejected.text).0.to_string();
    match first.code {
        Code::LegacyVersion => (SourceStatus::Legacy, SourceError::Legacy),
        Code::UnsupportedVersion => (SourceStatus::Invalid, SourceError::UnsupportedVersion),
        Code::Parse => (SourceStatus::Invalid, SourceError::NotToml { line }),
        Code::UnknownKey => (SourceStatus::Invalid, SourceError::UnknownKey { line }),
        Code::ExtendsCycle => (SourceStatus::Invalid, SourceError::ExtendsLoop { line }),
        Code::ExtendsMissing => (SourceStatus::Invalid, SourceError::ExtendsNoPlugin { line }),
        Code::BudgetExceeded
            if first.data.as_deref().is_some_and(|d| {
                matches!(
                    d.budget,
                    ConfigBudget::ExtendsDepth
                        | ConfigBudget::ExtendsFiles
                        | ConfigBudget::ExtendsBytes
                        | ConfigBudget::ExtendsAppliedBytes
                )
            }) =>
        {
            (SourceStatus::Invalid, SourceError::ExtendsBudget { line })
        }
        _ => (SourceStatus::Invalid, SourceError::Invalid { line }),
    }
}
