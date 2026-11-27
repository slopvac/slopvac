//! `slopvac report consent on|off|show` and the per-user record behind it.
//!
//! Contract: `docs/contract/policies.md`, sections 1.4, 1.5 and 1.8. The record is
//! `reporting.json` in the state directory, mode 0600, at most 64 KiB. Only `consent on` and
//! `consent off` write it, under the guard file `reporting.json.lock`. A record that cannot be
//! read is never rewritten: the command exits 2 with `report-state-failed`, and the user
//! removes the file to start again.

use std::fs::{DirBuilder, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use clap::{ArgMatches, Command};
use serde::{Deserialize, Serialize};
use slopvac_fs::{InputKind, ReadOptions, WriteOptions};
use slopvac_messages::Catalog;

use super::messages::ReportError;
use crate::env::Env;
use crate::errors::CliError;
use crate::exit;

/// The record file in the state directory.
const RECORD_FILE: &str = "reporting.json";
/// The guard file that serialises writers.
const LOCK_FILE: &str = "reporting.json.lock";
/// The most bytes of the record that are read. A larger file is damaged.
const MAX_RECORD_BYTES: u64 = 64 * 1024;
/// The only schema version this build reads and writes.
const SCHEMA: u32 = 1;

/// The user's answer to "may agents report slopvac problems for me".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Consent {
    On,
    Off,
}

impl Consent {
    const fn word(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

/// The content of `reporting.json`. Every field is required, and an unknown one is refused.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    consent: Consent,
    /// When the consent was recorded, in Unix seconds.
    recorded: u64,
    /// Unix seconds of earlier unattended filings. `consent on|off` keeps them as they are.
    unattended: Vec<u64>,
}

/// Why the record cannot be used. Each variant is a fixed phrase for `report-state-failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateProblem {
    Damaged,
    TooLarge,
    NotRegular,
    Unreadable,
    Unwritable,
    Directory,
    Clock,
}

impl StateProblem {
    const fn phrase(self) -> &'static str {
        match self {
            Self::Damaged => "it is damaged",
            Self::TooLarge => "it is larger than 64 KiB",
            Self::NotRegular => "it is not a regular file",
            Self::Unreadable => "it cannot be read",
            Self::Unwritable => "it cannot be written",
            Self::Directory => "the state directory cannot be found",
            Self::Clock => "the clock is set before 1970",
        }
    }
}

impl From<StateProblem> for CliError {
    fn from(problem: StateProblem) -> Self {
        ReportError::StateFailed {
            reason: problem.phrase().to_owned(),
        }
        .into()
    }
}

/// What `show` and `on|off` print.
#[derive(Debug, Catalog)]
enum ConsentText {
    /// No consent is recorded.
    #[code("report-consent-unset")]
    #[text("Issue reporting consent: unset.")]
    Unset,
    /// The recorded consent and when it was recorded.
    #[code("report-consent-recorded")]
    #[text("Issue reporting consent: {value}, recorded at Unix time {recorded}.")]
    Recorded {
        /// `on` or `off`.
        value: String,
        /// Unix seconds, as digits.
        recorded: String,
    },
    /// A new consent was recorded.
    #[code("report-consent-set")]
    #[text("Issue reporting consent is now {value}.")]
    Set {
        /// `on` or `off`.
        value: String,
    },
}

/// The catalog entries of the text `consent` prints, for the message registry.
#[must_use]
pub fn text_entries() -> &'static [slopvac_messages::Entry] {
    ConsentText::entries()
}

/// The `consent` subcommand of `report`.
#[must_use]
pub fn command() -> Command {
    Command::new("consent")
        .about("Record whether agents may report slopvac problems for you.")
        .long_about(
            "Record, for your user account, whether an agent may report slopvac problems. `on` and \
             `off` write the answer to a file in the state directory, and `show` prints it with the \
             time it was recorded. The answer is unset until you record one. The command refuses \
             to change the answer in CI.",
        )
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("on").about("Allow agents to report slopvac problems."))
        .subcommand(Command::new("off").about("Do not allow agents to report slopvac problems."))
        .subcommand(
            Command::new("show").about("Print the recorded answer and when it was recorded."),
        )
}

/// Whether the process runs in CI. `CI` counts with any value, the same test as the update
/// check (`silenced` in `slopvac-update/src/notice.rs`). `GITHUB_ACTIONS` counts the same way:
/// a workflow can override `CI`, but not `GITHUB_*`.
fn in_ci(env: &dyn Env) -> bool {
    env.var("CI").is_some() || env.var("GITHUB_ACTIONS").is_some()
}

/// Run `report consent`. `clock` gives the time in Unix seconds, or `None` before 1970.
///
/// # Errors
/// `report-ci-refused` for `on` or `off` in CI, and `report-state-failed` when the record cannot
/// be used.
pub fn run(
    m: &ArgMatches,
    env: &dyn Env,
    clock: &dyn Fn() -> Option<u64>,
    out: &mut dyn std::io::Write,
) -> Result<u8, CliError> {
    let wanted = match m.subcommand_name() {
        Some("on") => Some(Consent::On),
        Some("off") => Some(Consent::Off),
        _ => None,
    };
    if wanted.is_some() && in_ci(env) {
        // Before anything touches the state directory.
        return Err(ReportError::CiRefused.into());
    }
    let store = Store::new(state_dir(env)?);
    let text = if let Some(consent) = wanted {
        let now = clock().ok_or(StateProblem::Clock)?;
        store.set(consent, now)?;
        ConsentText::Set {
            value: consent.word().to_owned(),
        }
    } else {
        match store.read()? {
            None => ConsentText::Unset,
            Some(record) => ConsentText::Recorded {
                value: record.consent.word().to_owned(),
                recorded: record.recorded.to_string(),
            },
        }
    };
    super::write_all(
        out,
        format!("{}\n", slopvac_messages::render(&text)).as_bytes(),
    )?;
    Ok(exit::PASS)
}

fn state_dir(env: &dyn Env) -> Result<PathBuf, StateProblem> {
    let lookup = |name: &str| env.var(name).ok_or(std::env::VarError::NotPresent);
    slopvac_fs::state_dir_with(&lookup, &std::env::home_dir).map_err(|e| {
        tracing::debug!(error = %e, "the state directory cannot be resolved");
        StateProblem::Directory
    })
}

/// Log a failure without a path and map it to [`StateProblem::Unwritable`].
fn unwritable(what: &'static str) -> impl FnOnce(impl std::fmt::Display) -> StateProblem {
    move |e| {
        tracing::debug!(error = %e, "{what}");
        StateProblem::Unwritable
    }
}

/// The record in one state directory.
struct Store {
    dir: PathBuf,
}

impl Store {
    const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The record, or `None` when no file exists. Creates nothing.
    fn read(&self) -> Result<Option<Record>, StateProblem> {
        let path = self.dir.join(RECORD_FILE);
        let opts = ReadOptions::new().reject_multiply_linked(true);
        let bytes = match slopvac_fs::read_capped_regular(&path, MAX_RECORD_BYTES, opts) {
            Ok(bytes) => bytes,
            Err(e) => {
                return match e.input_kind() {
                    Some(InputKind::NotFound) => Ok(None),
                    Some(InputKind::TooLarge) => Err(StateProblem::TooLarge),
                    Some(
                        InputKind::Symlink | InputKind::NotRegular | InputKind::MultiplyLinked,
                    ) => Err(StateProblem::NotRegular),
                    _ => {
                        tracing::debug!(error = %e, "the reporting record cannot be read");
                        Err(StateProblem::Unreadable)
                    }
                };
            }
        };
        let record: Record = serde_json::from_slice(&bytes).map_err(|e| {
            tracing::debug!(error = %e, "the reporting record does not parse");
            StateProblem::Damaged
        })?;
        if record.schema == SCHEMA {
            Ok(Some(record))
        } else {
            Err(StateProblem::Damaged)
        }
    }

    /// Record `consent` at `now`, keeping the `unattended` entries of the current record.
    ///
    /// Under the guard lock: read the record, refuse a damaged one, then replace the file.
    fn set(&self, consent: Consent, now: u64) -> Result<(), StateProblem> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .map_err(unwritable("the state directory cannot be created"))?;
        let _guard = self
            .lock()
            .map_err(unwritable("the guard file cannot be locked"))?;
        let unattended = self.read()?.map(|r| r.unattended).unwrap_or_default();
        let record = Record {
            schema: SCHEMA,
            consent,
            recorded: now,
            unattended,
        };
        let mut bytes = serde_json::to_vec_pretty(&record)
            .map_err(unwritable("the record cannot be encoded"))?;
        bytes.push(b'\n');
        let dir = cap_std::fs::Dir::open_ambient_dir(&self.dir, cap_std::ambient_authority())
            .map_err(unwritable("the state directory cannot be opened"))?;
        slopvac_fs::write_atomic(
            &dir,
            Path::new(RECORD_FILE),
            &bytes,
            WriteOptions::new().new_file_mode(0o600),
        )
        .map_err(unwritable("the record cannot be written"))
    }

    /// Take the exclusive guard lock. It is released when the returned file drops.
    fn lock(&self) -> std::io::Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(self.dir.join(LOCK_FILE))?;
        file.lock()?;
        Ok(file)
    }
}
