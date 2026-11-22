//! The `slopvac plugins` command group: its clap tree and the subcommands that run on the
//! library alone.

use std::io::Write;
use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgMatches, Command};
use slopvac_fs::{InputKind, ReadOptions, read_capped_regular_to_string};
use slopvac_messages::render;
use slopvac_sanitize::line;

use crate::lock::{Lockfile, MAX_LOCK_BYTES};
use crate::messages::cli::{CliText, RunError};
use crate::sync::{PackState, Store, status};
use crate::trust::TrustStore;

/// Where `slopvac plugins` reads and writes. The caller resolves the directories: the cache
/// and state directories belong to the user, never to the project.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Dirs {
    /// The project root, which holds `.slopvac/plugins.lock`.
    pub project_root: PathBuf,
    /// The per-user directory that holds installed plugin trees.
    pub cache_dir: PathBuf,
    /// The per-user directory that holds the trust store.
    pub state_dir: PathBuf,
}

impl Dirs {
    /// The three directories.
    #[must_use]
    pub fn new(
        project_root: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            project_root: project_root.into(),
            cache_dir: cache_dir.into(),
            state_dir: state_dir.into(),
        }
    }
}

fn arg(id: &'static str, help: &CliText) -> Arg {
    Arg::new(id).help(render(help)).required(true)
}

fn registry() -> Arg {
    Arg::new("registry")
        .long("registry")
        .value_name("HOST")
        .help(render(&CliText::ArgRegistry))
}

fn path_arg() -> Arg {
    Arg::new("path")
        .value_name("PATH")
        .help(render(&CliText::ArgPath))
}

/// The `plugins` command group.
#[must_use]
pub fn command() -> Command {
    let sub = |name: &'static str, about: &CliText| Command::new(name).about(render(about));
    Command::new("plugins")
        .about(render(&CliText::HelpAbout))
        .subcommand(sub("list", &CliText::HelpList))
        .subcommand(
            sub("search", &CliText::HelpSearch)
                .arg(arg("query", &CliText::ArgQuery))
                .arg(
                    Arg::new("index")
                        .long("index")
                        .value_name("NAME")
                        .help(render(&CliText::ArgIndex)),
                ),
        )
        .subcommand(sub("sync", &CliText::HelpSync))
        .subcommand(sub("status", &CliText::HelpStatus))
        .subcommand(sub("outdated", &CliText::HelpOutdated))
        .subcommand(
            sub("update", &CliText::HelpUpdate).arg(
                Arg::new("id")
                    .num_args(0..)
                    .action(ArgAction::Append)
                    .help(render(&CliText::ArgIds)),
            ),
        )
        .subcommand(sub("trust", &CliText::HelpTrust).arg(arg("id", &CliText::ArgId)))
        .subcommand(sub("untrust", &CliText::HelpUntrust).arg(arg("id", &CliText::ArgId)))
        .subcommand(
            sub("pin", &CliText::HelpPin)
                .arg(arg("namespace", &CliText::ArgNamespace))
                .arg(arg("fingerprint", &CliText::ArgFingerprint))
                .arg(registry()),
        )
        .subcommand(
            sub("rotate", &CliText::HelpRotate)
                .arg(arg("namespace", &CliText::ArgNamespace))
                .arg(registry()),
        )
        .subcommand(sub("new", &CliText::HelpNew).arg(arg("name", &CliText::ArgName)))
        .subcommand(sub("validate", &CliText::HelpValidate).arg(path_arg()))
        .subcommand(sub("test", &CliText::HelpTest).arg(path_arg()))
        .subcommand(sub("build", &CliText::HelpBuild).arg(path_arg()))
        .subcommand(sub("publish", &CliText::HelpPublish).arg(path_arg()))
}

impl RunError {
    /// The exit status of a failed command. Every failure of the group is an incomplete run.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        2
    }
}

/// Run the subcommand that `matches` selects, writing its report to `out`. Returns the exit
/// status: 0 when the check passed, 2 when the report shows an incomplete state.
///
/// Without a subcommand it prints the help.
///
/// # Errors
/// A [`RunError`], which exits 2. A subcommand with no implementation in this build fails
/// with `plugins-command-unavailable`.
pub fn run(matches: &ArgMatches, dirs: &Dirs, out: &mut dyn Write) -> Result<u8, RunError> {
    match matches.subcommand() {
        Some(("status", _)) => run_status(dirs, out),
        Some(("untrust", m)) => {
            let id = m.get_one::<String>("id").map_or("", String::as_str);
            run_untrust(dirs, id, out)
        }
        // STUB(slopvac-8l4.12.57): these need the resolved config (ranges, sources, indexes)
        // and the network settings, which the binary wiring supplies.
        // STUB(slopvac-8l4.9.62): `search` also needs the index fetch and merge.
        // STUB(slopvac-8l4.9.61): `pin` and `rotate` need per-registry pins, and the library
        // pins a key, not a fingerprint.
        // STUB(slopvac-8l4.9.63): the authoring commands.
        Some((name, _)) => Err(RunError::Unavailable {
            command: name.to_owned(),
        }),
        None => write(out, command().render_help().to_string().as_bytes()).map(|()| 0),
    }
}

fn write(out: &mut dyn Write, bytes: &[u8]) -> Result<(), RunError> {
    out.write_all(bytes).map_err(|e| RunError::Output {
        detail: e.kind().to_string(),
    })
}

fn lock_failed(detail: &impl std::fmt::Display) -> RunError {
    RunError::LockUnreadable {
        detail: detail.to_string(),
    }
}

fn read_lock(dirs: &Dirs) -> Result<Lockfile, RunError> {
    let path = dirs.project_root.join(".slopvac/plugins.lock");
    match read_capped_regular_to_string(&path, MAX_LOCK_BYTES as u64, ReadOptions::default()) {
        Ok(text) => Lockfile::parse(&text).map_err(|e| lock_failed(&e)),
        Err(e) if e.input_kind() == Some(InputKind::NotFound) => {
            Lockfile::new(Vec::new()).map_err(|e| lock_failed(&e))
        }
        Err(e) => Err(lock_failed(&e)),
    }
}

fn run_status(dirs: &Dirs, out: &mut dyn Write) -> Result<u8, RunError> {
    let lock = read_lock(dirs)?;
    let store = Store::new(&dirs.cache_dir);
    let statuses =
        status(&store, &dirs.project_root, &lock).map_err(|e| RunError::StoreFailed {
            detail: e.to_string(),
        })?;
    let mut complete = true;
    for s in &statuses {
        let (id, version) = (line(s.id.as_str()), s.version.to_string());
        let text = match s.state {
            PackState::Recorded(_) | PackState::InPlace(_) => {
                CliText::StatusInstalled { id, version }
            }
            PackState::NotInstalled => CliText::StatusNotInstalled { id, version },
            PackState::Tampered(_) | PackState::InPlaceChanged(_) => {
                CliText::StatusChanged { id, version }
            }
        };
        complete &= s.is_recorded();
        write(out, format!("{}\n", render(&text)).as_bytes())?;
    }
    Ok(if complete { 0 } else { 2 })
}

fn run_untrust(dirs: &Dirs, id: &str, out: &mut dyn Write) -> Result<u8, RunError> {
    let store_failed = |e: crate::trust::TrustError| RunError::StoreFailed {
        detail: e.to_string(),
    };
    let store = TrustStore::open(&dirs.state_dir);
    let hashes: Vec<String> = store
        .records()
        .map_err(store_failed)?
        .into_iter()
        .filter(|r| r.name == id)
        .map(|r| r.hash)
        .collect();
    if hashes.is_empty() {
        return Err(RunError::NotTrusted { id: line(id) });
    }
    for hash in hashes {
        let digest = crate::oci::Digest::parse(&hash).map_err(|e| RunError::StoreFailed {
            detail: e.to_string(),
        })?;
        store.forget(&digest).map_err(store_failed)?;
    }
    write(
        out,
        format!("{}\n", render(&CliText::Untrusted { id: line(id) })).as_bytes(),
    )?;
    Ok(0)
}
