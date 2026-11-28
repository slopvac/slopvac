//! `slopvac report consent on|off|show` and the per-user record behind it.
//!
//! Contract: `docs/contract/policies.md`, sections 1.4, 1.5 and 1.8. The record is
//! `reporting.json` in the state directory, mode 0600, at most 64 KiB. Only `consent on` and
//! `consent off` write it, under the guard file `reporting.json.lock`. A record that cannot be
//! read is never rewritten: the command exits 2 with `report-state-failed`, and the user
//! removes the file to start again.

use std::ffi::OsStr;
use std::fs::{DirBuilder, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use cap_std::fs::Dir;
use clap::{ArgMatches, Command};
use serde::{Deserialize, Serialize};
use slopvac_fs::{InputKind, ReadOptions, WriteOptions};
use slopvac_messages::Catalog;

use super::messages::{ReportError, ReportPhrase};
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

/// Why the record cannot be used. Each variant is a fixed phrase for the `reason` of
/// `report-state-failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Catalog)]
enum StateProblem {
    /// The record does not parse, or it lacks the shape of schema 1.
    #[code("report-state-damaged")]
    #[text("it is damaged")]
    Damaged,
    /// The record is over the cap.
    #[code("report-state-too-large")]
    #[text("it is larger than 64 KiB")]
    TooLarge,
    /// The new record would be over the cap.
    #[code("report-state-rewrite-too-large")]
    #[text("the new record would be larger than 64 KiB")]
    WouldExceed,
    /// The path is a link, a directory or another special file.
    #[code("report-state-not-regular")]
    #[text("it is not a regular file")]
    NotRegular,
    /// The record could not be read.
    #[code("report-state-unreadable")]
    #[text("it cannot be read")]
    Unreadable,
    /// The record could not be written.
    #[code("report-state-unwritable")]
    #[text("it cannot be written")]
    Unwritable,
    /// Another process changed the record while the command ran.
    #[code("report-state-changed")]
    #[text("it changed while the command ran")]
    Changed,
    /// The state directory is not known.
    #[code("report-state-no-directory")]
    #[text("the state directory cannot be found")]
    Directory,
    /// The clock cannot give a time.
    #[code("report-state-clock")]
    #[text("the clock is set before 1970")]
    Clock,
}

impl From<StateProblem> for CliError {
    fn from(problem: StateProblem) -> Self {
        ReportError::StateFailed {
            reason: slopvac_messages::render(&problem),
        }
        .into()
    }
}

/// The catalog entries of the reasons `report-state-failed` gives, for the message registry.
#[must_use]
pub fn state_entries() -> &'static [slopvac_messages::Entry] {
    StateProblem::entries()
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
        .about(ReportPhrase::ConsentAbout.as_static())
        .long_about(ReportPhrase::ConsentLongAbout.as_static())
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("on").about(ReportPhrase::ConsentOnAbout.as_static()))
        .subcommand(Command::new("off").about(ReportPhrase::ConsentOffAbout.as_static()))
        .subcommand(Command::new("show").about(ReportPhrase::ConsentShowAbout.as_static()))
}

/// Whether the process runs in CI. `CI` counts with any value, the same test as the update
/// check (`silenced` in `slopvac-update/src/notice.rs`), and a value that is not UTF-8 counts
/// too. `GITHUB_ACTIONS` counts the same way: a workflow can override `CI`, but not `GITHUB_*`.
fn in_ci(env: &dyn Env) -> bool {
    env.var_is_set("CI") || env.var_is_set("GITHUB_ACTIONS")
}

/// The refusal for a `report consent on` or `off` in CI, found in the parsed command line.
///
/// The caller runs this before it reads any project file, so the refusal never depends on one.
#[must_use]
pub fn ci_refusal(root: &ArgMatches, env: &dyn Env) -> Option<CliError> {
    let consent = root
        .subcommand_matches("report")?
        .subcommand_matches("consent")?;
    (matches!(consent.subcommand_name(), Some("on" | "off")) && in_ci(env))
        .then(|| ReportError::CiRefused.into())
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
fn unwritable<E: std::fmt::Display>(what: &'static str) -> impl FnOnce(E) -> StateProblem {
    move |e| {
        tracing::debug!(error = %e, "{what}");
        StateProblem::Unwritable
    }
}

/// Map a failed confined read: a missing file is `Ok(None)` for the caller to handle.
fn classify(e: &slopvac_fs::Error) -> Option<StateProblem> {
    match e.input_kind() {
        Some(InputKind::NotFound) => None,
        Some(InputKind::TooLarge) => Some(StateProblem::TooLarge),
        Some(InputKind::Symlink | InputKind::NotRegular | InputKind::MultiplyLinked) => {
            Some(StateProblem::NotRegular)
        }
        _ => {
            tracing::debug!(error = %e, "the reporting record cannot be read");
            Some(StateProblem::Unreadable)
        }
    }
}

/// Parse the bytes of a record. The log gets the kind of failure and its position, never the
/// text of the record: a parse error quotes the value it stopped at.
fn parse(bytes: &[u8]) -> Result<Record, StateProblem> {
    // A struct also reads from a JSON array, so the top level is checked on its own.
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return Err(StateProblem::Damaged);
    }
    let record: Record = serde_json::from_slice(bytes).map_err(|e| {
        tracing::debug!(
            category = ?e.classify(),
            line = e.line(),
            column = e.column(),
            "the reporting record does not parse"
        );
        StateProblem::Damaged
    })?;
    if record.schema == SCHEMA {
        Ok(record)
    } else {
        Err(StateProblem::Damaged)
    }
}

/// Whether the record at `name` still holds exactly the bytes that were validated.
fn unchanged(dir: &Dir, name: &OsStr, seen: Option<&[u8]>) -> bool {
    match slopvac_fs::read_capped_regular_in(dir, Path::new(name), MAX_RECORD_BYTES) {
        Ok(now) => seen == Some(now.as_slice()),
        Err(e) if e.input_kind() == Some(InputKind::NotFound) => seen.is_none(),
        Err(_) => false,
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
        match slopvac_fs::read_capped_regular(&path, MAX_RECORD_BYTES, opts) {
            Ok(bytes) => parse(&bytes).map(Some),
            Err(e) => classify(&e).map_or(Ok(None), Err),
        }
    }

    /// Record `consent` at `now`, keeping the `unattended` entries of the current record.
    ///
    /// Under the guard lock: read the record through one directory handle, refuse a damaged
    /// one, then replace it only if it still holds the bytes that were
    /// read.
    fn set(&self, consent: Consent, now: u64) -> Result<(), StateProblem> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .map_err(unwritable("the state directory cannot be created"))?;
        let _guard = self
            .lock()
            .map_err(unwritable("the guard file cannot be locked"))?;
        let dir = Dir::open_ambient_dir(&self.dir, cap_std::ambient_authority())
            .map_err(unwritable("the state directory cannot be opened"))?;
        let name = Path::new(RECORD_FILE);
        let seen = match slopvac_fs::read_capped_regular_in(&dir, name, MAX_RECORD_BYTES) {
            Ok(bytes) => Some(bytes),
            Err(e) => match classify(&e) {
                None => None,
                Some(problem) => return Err(problem),
            },
        };
        let unattended = seen
            .as_deref()
            .map(parse)
            .transpose()?
            .map(|r| r.unattended)
            .unwrap_or_default();
        let record = Record {
            schema: SCHEMA,
            consent,
            recorded: now,
            unattended,
        };
        let mut bytes =
            serde_json::to_vec(&record).map_err(unwritable("the record cannot be encoded"))?;
        bytes.push(b'\n');
        if u64::try_from(bytes.len()).map_or(true, |n| n > MAX_RECORD_BYTES) {
            return Err(StateProblem::WouldExceed);
        }
        slopvac_fs::write_atomic_if(
            &dir,
            name,
            &bytes,
            WriteOptions::new().forced_mode(0o600),
            |d, file| Ok(unchanged(d, file, seen.as_deref())),
        )
        .map_err(|e| match e {
            slopvac_fs::Error::Precondition { .. } => StateProblem::Changed,
            other => unwritable("the record cannot be written")(other),
        })
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
