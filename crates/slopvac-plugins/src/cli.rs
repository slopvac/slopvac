//! The `slopvac plugins` command group: its clap tree, the context the binary builds and the
//! subcommands that run on the library.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command};
use slopvac_fs::{InputKind, ReadOptions, read_capped_regular_to_string};
use slopvac_messages::render;
use slopvac_sanitize::line;
use slopvac_update::messages::UpdateMessage;

use crate::PluginSpec;
use crate::lock::{
    LockedPack, Lockfile, MAX_LOCK_BYTES, PackId, PackKind, VersionReq, outdated as lock_outdated,
};
use crate::messages::cli::{CliText, Redacted, RunError};
use crate::oci::Digest;
use crate::pyenv::EnvStore;
use crate::revoke::{RevocationNotice, Revocations};
use crate::sign::KeyPolicy;
use crate::sync::{
    Change, Network, PackState, PackStatus, PythonBuild, PythonState, Store, Syncer, status,
    status_with_python, update_notices,
};
use crate::trust::{TrustError, TrustGate, TrustState, TrustStore, VerifiedPlugin};
use crate::update::{LISTING_BUDGET_BYTES, Outcome, Updater, list_candidates};

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

/// Loads the files of a locked code plugin, for `plugins trust` and the trust state in
/// `plugins status`. The binary implements it over the installed trees and the resolved
/// config; the library never decides which program a plugin runs.
pub trait PluginLoader {
    /// The verified plugin and the spec it would start with.
    ///
    /// # Errors
    /// The reason the plugin cannot be loaded, as one sentence with no document text.
    fn load(&self, id: &PackId) -> Result<(VerifiedPlugin, PluginSpec), String>;
}

/// Everything the commands need from outside the library: directories, the network, keys,
/// revocations and the inputs that only the resolved config knows. The binary builds it once.
#[derive(Clone)]
#[non_exhaustive]
pub struct Runner<'a> {
    dirs: Dirs,
    network: Network<'a>,
    policy: &'a KeyPolicy,
    revocations: &'a Revocations,
    now: String,
    ranges: BTreeMap<PackId, VersionReq>,
    configured: Vec<String>,
    core_version: String,
    trusted_env: Vec<Digest>,
    loader: Option<&'a dyn PluginLoader>,
    python: Option<PythonBuild<'a>>,
    envs: Option<&'a EnvStore>,
}

impl<'a> Runner<'a> {
    /// A runner over `dirs` and the network mode, with the keys and the revocation source.
    /// `now` is the clock as `YYYY-MM-DDTHH:MM:SSZ`, for trust records.
    #[must_use]
    pub fn new(
        dirs: Dirs,
        network: Network<'a>,
        policy: &'a KeyPolicy,
        revocations: &'a Revocations,
        now: impl Into<String>,
    ) -> Self {
        Self {
            dirs,
            network,
            policy,
            revocations,
            now: now.into(),
            ranges: BTreeMap::new(),
            configured: Vec::new(),
            core_version: String::new(),
            trusted_env: Vec::new(),
            loader: None,
            python: None,
            envs: None,
        }
    }

    /// The version ranges that the config sets, by plugin id. A plugin with no range is
    /// never updated.
    #[must_use]
    pub fn with_ranges(mut self, ranges: BTreeMap<PackId, VersionReq>) -> Self {
        self.ranges = ranges;
        self
    }

    /// The ids of the plugins that the config names, for `plugins list`.
    #[must_use]
    pub fn with_configured(mut self, ids: Vec<String>) -> Self {
        self.configured = ids;
        self
    }

    /// The version of the built-in core, which is the binary's own version.
    #[must_use]
    pub fn with_core_version(mut self, version: impl Into<String>) -> Self {
        self.core_version = version.into();
        self
    }

    /// The content hashes named in `SLOPVAC_TRUST`.
    #[must_use]
    pub fn with_trusted_env(mut self, hashes: Vec<Digest>) -> Self {
        self.trusted_env = hashes;
        self
    }

    /// The loader of code plugin files.
    #[must_use]
    pub fn with_loader(mut self, loader: &'a dyn PluginLoader) -> Self {
        self.loader = Some(loader);
        self
    }

    /// Where `sync` builds managed Python environments.
    #[must_use]
    pub fn with_python(mut self, python: PythonBuild<'a>) -> Self {
        self.python = Some(python);
        self
    }

    /// The store of managed Python environments, which `status` checks in full.
    #[must_use]
    pub fn with_envs(mut self, envs: &'a EnvStore) -> Self {
        self.envs = Some(envs);
        self
    }

    fn trust_store(&self) -> TrustStore {
        TrustStore::open(&self.dirs.state_dir)
    }

    fn gate(&self) -> TrustGate {
        TrustGate::new(self.trust_store(), self.trusted_env.clone(), Vec::new())
            .with_revocation_cache(self.revocations.clone())
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
pub fn run(matches: &ArgMatches, runner: &Runner<'_>, out: &mut dyn Write) -> Result<u8, RunError> {
    let id_of = |m: &ArgMatches| m.get_one::<String>("id").cloned().unwrap_or_default();
    match matches.subcommand() {
        Some(("list", _)) => run_list(runner, out),
        Some(("status", _)) => run_status(runner, out),
        Some(("sync", _)) => run_sync(runner, out),
        Some(("outdated", _)) => run_outdated(runner, out),
        Some(("update", m)) => {
            let only: Vec<String> = m
                .get_many::<String>("id")
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            run_update(runner, &only, out)
        }
        Some(("trust", m)) => run_trust(runner, &id_of(m), out),
        Some(("untrust", m)) => run_untrust(runner, &id_of(m), out),
        // STUB(slopvac-8l4.9.62): `search` needs the index fetch, verify, cache and merge.
        // STUB(slopvac-8l4.9.61): `pin` and `rotate` need per-registry pins, and the library
        // pins a key, not a fingerprint.
        // STUB(slopvac-8l4.9.63): the authoring commands.
        Some((name, _)) => Err(RunError::NotWired {
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

fn say(out: &mut dyn Write, text: &CliText) -> Result<(), RunError> {
    write(out, format!("{}\n", render(text)).as_bytes())
}

fn notices(out: &mut dyn Write, list: &[RevocationNotice]) -> Result<(), RunError> {
    for n in list {
        say(
            out,
            &CliText::NoticeLine {
                code: n.code().to_owned(),
                message: line(&n.to_string()),
            },
        )?;
    }
    Ok(())
}

fn read_lock(runner: &Runner<'_>) -> Result<Lockfile, RunError> {
    let path = runner.dirs.project_root.join(".slopvac/plugins.lock");
    let invalid = |source: crate::lock::LockError| RunError::LockInvalid {
        detail: source.to_string(),
        source,
    };
    match read_capped_regular_to_string(&path, MAX_LOCK_BYTES as u64, ReadOptions::default()) {
        Ok(text) => Lockfile::parse(&text).map_err(invalid),
        Err(e) if e.input_kind() == Some(InputKind::NotFound) => {
            Lockfile::new(Vec::new()).map_err(invalid)
        }
        Err(source) => Err(RunError::LockUnreadable {
            detail: source.to_string(),
            source,
        }),
    }
}

fn kind_name(kind: PackKind) -> String {
    kind.as_str().to_owned()
}

/// `text` as a store failure, with every path shown under the project root.
fn store_failed(runner: &Runner<'_>, e: &TrustError) -> RunError {
    let root = &runner.dirs.project_root;
    let mut detail = e.to_string();
    let shown = |p: &Path| slopvac_fs::display_under_root(p, root);
    if let TrustError::Corrupt { path, .. } = e {
        detail = detail.replace(&line(&path.to_string_lossy()), &shown(path));
    }
    for dir in [&runner.dirs.state_dir, &runner.dirs.cache_dir] {
        detail = detail.replace(&line(&dir.to_string_lossy()), &shown(dir));
    }
    RunError::Store {
        source: Redacted(detail.clone()),
        detail,
    }
}

fn revoked(runner: &Runner<'_>, pack: &LockedPack, list: &crate::revoke::RevocationList) -> bool {
    let _ = runner;
    list.revokes_digest(pack.digest.as_str())
        || pack
            .signature
            .as_ref()
            .is_some_and(|s| list.revokes_key(s.key_id.as_str()))
}

fn trust_part(runner: &Runner<'_>, pack: &LockedPack) -> CliText {
    let id = pack.id.clone();
    let Some(loader) = runner.loader else {
        return CliText::PartTrustUnknown {
            why: "the plugin files are not loaded".to_owned(),
        };
    };
    let state = loader
        .load(&id)
        .map_err(|why| line(&why))
        .and_then(|(vp, spec)| {
            runner
                .gate()
                .state(&vp, &spec)
                .map(|(_, s)| s)
                .map_err(|e| store_failed(runner, &e).to_string())
        });
    match state {
        Ok(TrustState::Trusted) => CliText::PartTrusted,
        Ok(TrustState::TrustedByEnv) => CliText::PartTrustedEnv,
        Ok(TrustState::Revoked) => CliText::PartTrustRevoked,
        Ok(TrustState::Untrusted { .. }) => CliText::PartUntrusted {
            id: id.as_str().to_owned(),
        },
        Err(why) => CliText::PartTrustUnknown { why },
    }
}

fn state_part(s: &PackStatus) -> CliText {
    match s.state {
        PackState::Recorded(_) | PackState::InPlace(_) => CliText::PartInstalled,
        PackState::NotInstalled => CliText::PartNotInstalled,
        PackState::Tampered(_) | PackState::InPlaceChanged(_) => CliText::PartChanged,
    }
}

fn statuses(runner: &Runner<'_>, lock: &Lockfile) -> Result<Vec<PackStatus>, RunError> {
    let store = Store::new(&runner.dirs.cache_dir);
    let root = &runner.dirs.project_root;
    match runner.envs {
        Some(envs) => status_with_python(&store, envs, root, lock),
        None => status(&store, root, lock),
    }
    .map_err(|source| RunError::Sync {
        detail: source.to_string(),
        source: Box::new(source),
    })
}

fn join(parts: &[CliText]) -> String {
    parts
        .iter()
        .map(|p| render(p))
        .collect::<Vec<_>>()
        .join("; ")
}

fn run_status(runner: &Runner<'_>, out: &mut dyn Write) -> Result<u8, RunError> {
    let lock = read_lock(runner)?;
    let statuses = statuses(runner, &lock)?;
    let current = runner
        .revocations
        .cached()
        .map_err(|source| RunError::Revocation {
            detail: source.to_string(),
            source: Box::new(source),
        })?;
    let mut complete = true;
    for s in &statuses {
        let Some(pack) = lock.get(&s.id) else {
            continue;
        };
        let mut parts = vec![state_part(s)];
        let is_revoked = revoked(runner, pack, &current.list);
        if is_revoked {
            parts.push(CliText::PartRevoked);
        }
        if pack.kind == PackKind::Code {
            parts.push(trust_part(runner, pack));
        }
        match &s.python {
            PythonState::NotChecked => {}
            PythonState::Verified(_) => parts.push(CliText::PartPythonVerified),
            PythonState::Missing => parts.push(CliText::PartPythonMissing),
            PythonState::Failed(why) => parts.push(CliText::PartPythonFailed { why: line(why) }),
        }
        complete &= s.is_recorded()
            && !is_revoked
            && !matches!(s.python, PythonState::Missing | PythonState::Failed(_));
        say(
            out,
            &CliText::Line {
                id: line(s.id.as_str()),
                version: s.version.to_string(),
                kind: kind_name(pack.kind),
                parts: join(&parts),
            },
        )?;
    }
    notices(out, &current.notices)?;
    Ok(if complete { 0 } else { 2 })
}

fn run_list(runner: &Runner<'_>, out: &mut dyn Write) -> Result<u8, RunError> {
    let lock = read_lock(runner)?;
    say(
        out,
        &CliText::ListCore {
            version: line(&runner.core_version),
        },
    )?;
    for s in statuses(runner, &lock)? {
        let Some(pack) = lock.get(&s.id) else {
            continue;
        };
        say(
            out,
            &CliText::Line {
                id: line(s.id.as_str()),
                version: s.version.to_string(),
                kind: kind_name(pack.kind),
                parts: render(&state_part(&s)),
            },
        )?;
    }
    for id in &runner.configured {
        if id != "core" && !lock.packs().iter().any(|p| p.id.as_str() == id) {
            say(out, &CliText::ListNotLocked { id: line(id) })?;
        }
    }
    Ok(0)
}

fn run_sync(runner: &Runner<'_>, out: &mut dyn Write) -> Result<u8, RunError> {
    let lock = read_lock(runner)?;
    let store = Store::new(&runner.dirs.cache_dir);
    let mut syncer = Syncer::new(
        &store,
        runner.network,
        runner.policy,
        &runner.dirs.project_root,
        runner.revocations,
    );
    if let Some(python) = runner.python {
        syncer = syncer.with_python(python);
    }
    let report = syncer.sync(&lock).map_err(|source| RunError::Sync {
        detail: source.to_string(),
        source: Box::new(source),
    })?;
    for p in &report.packs {
        let change = match p.change {
            Change::Installed => CliText::ChangeInstalled,
            Change::Unchanged => CliText::ChangeUnchanged,
            Change::Reinstalled => CliText::ChangeReinstalled,
            Change::InPlace => CliText::ChangeInPlace,
        };
        say(
            out,
            &CliText::SyncLine {
                id: line(p.id.as_str()),
                version: p.version.to_string(),
                change: render(&change),
            },
        )?;
    }
    notices(out, &report.notices)?;
    Ok(0)
}

fn run_outdated(runner: &Runner<'_>, out: &mut dyn Write) -> Result<u8, RunError> {
    let Network::Online(client) = runner.network else {
        return Err(RunError::Offline {
            source: UpdateMessage::OfflineNetworkNeeded {
                command: "plugins outdated".to_owned(),
            },
        });
    };
    let lock = read_lock(runner)?;
    let listed = list_candidates(
        client,
        &lock,
        &runner.ranges,
        |_| true,
        LISTING_BUDGET_BYTES,
    );
    for line_text in update_notices(&lock, &runner.ranges, &listed.available) {
        write(out, format!("{line_text}\n").as_bytes())?;
    }
    for (id, e) in &listed.results {
        say(
            out,
            &CliText::OutdatedFailed {
                id: line(id.as_str()),
                reason: line(&e.to_string()),
            },
        )?;
    }
    // Keep the pure query in the path, so the lines above and this check agree.
    let _ = lock_outdated(&lock, &runner.ranges, &listed.available);
    Ok(if listed.results.is_empty() { 0 } else { 2 })
}

fn run_update(runner: &Runner<'_>, only: &[String], out: &mut dyn Write) -> Result<u8, RunError> {
    let mut ids = Vec::with_capacity(only.len());
    for text in only {
        ids.push(
            text.parse::<PackId>()
                .map_err(|_| RunError::UnknownPlugin { id: line(text) })?,
        );
    }
    let report = Updater::new(runner.network, runner.policy, runner.revocations)
        .update(&runner.dirs.project_root, &runner.ranges, &ids)
        .map_err(|source| RunError::Update {
            detail: source.to_string(),
            source: Box::new(source),
        })?;
    for p in &report.packs {
        let id = line(p.id.as_str());
        let text = match &p.outcome {
            Outcome::Updated { to } => CliText::UpdateUpdated {
                id: id.clone(),
                from: p.locked.to_string(),
                to: to.to_string(),
            },
            Outcome::Unchanged => CliText::UpdateUnchanged {
                id: id.clone(),
                version: p.locked.to_string(),
            },
            Outcome::Refused(e) => CliText::UpdateRefused {
                id: id.clone(),
                reason: line(&e.to_string()),
            },
        };
        say(out, &text)?;
        if let Some(v) = &p.outside_range {
            say(
                out,
                &CliText::UpdateOutsideRange {
                    id,
                    version: v.to_string(),
                },
            )?;
        }
    }
    notices(out, &report.notices)?;
    Ok(report.exit_code())
}

/// The locked kind of `id`, or `None` when the lock does not hold it.
fn locked_kind(lock: &Lockfile, id: &str) -> Option<PackKind> {
    lock.packs()
        .iter()
        .find(|p| p.id.as_str() == id)
        .map(|p| p.kind)
}

fn run_trust(runner: &Runner<'_>, id: &str, out: &mut dyn Write) -> Result<u8, RunError> {
    let lock = read_lock(runner)?;
    match locked_kind(&lock, id) {
        None => return Err(RunError::UnknownPlugin { id: line(id) }),
        Some(PackKind::Data) => return Err(RunError::NotCode { id: line(id) }),
        Some(PackKind::Code) => {}
    }
    let refused = |detail: String| RunError::TrustInputs {
        id: line(id),
        detail: line(&detail),
    };
    let loader = runner
        .loader
        .ok_or_else(|| refused("the plugin files are not loaded".to_owned()))?;
    let pack_id = id.parse::<PackId>().map_err(|e| refused(e.to_string()))?;
    let (vp, spec) = loader.load(&pack_id).map_err(refused)?;
    let gate = runner.gate();
    let (_, state) = gate
        .state(&vp, &spec)
        .map_err(|e| store_failed(runner, &e))?;
    if state == TrustState::Revoked {
        return Err(RunError::Revoked { id: line(id) });
    }
    let record = gate
        .record_trust(&vp, &spec, &runner.now)
        .map_err(|e| store_failed(runner, &e))?;
    say(
        out,
        &CliText::Trusted {
            id: line(id),
            hash: record.hash,
        },
    )?;
    say(out, &CliText::TrustNotContainment)?;
    Ok(0)
}

fn run_untrust(runner: &Runner<'_>, id: &str, out: &mut dyn Write) -> Result<u8, RunError> {
    let lock = read_lock(runner)?;
    if locked_kind(&lock, id) == Some(PackKind::Data) {
        return Err(RunError::NotCode { id: line(id) });
    }
    let store = runner.trust_store();
    let hashes: Vec<String> = store
        .records()
        .map_err(|e| store_failed(runner, &e))?
        .into_iter()
        .filter(|r| r.name == id)
        .map(|r| r.hash)
        .collect();
    if hashes.is_empty() {
        return Err(RunError::NotTrusted { id: line(id) });
    }
    for hash in hashes {
        let digest = Digest::parse(&hash).map_err(|e| {
            store_failed(
                runner,
                &TrustError::Corrupt {
                    path: runner.dirs.state_dir.join("trust.json"),
                    why: e.to_string(),
                },
            )
        })?;
        store
            .forget(&digest)
            .map_err(|e| store_failed(runner, &e))?;
    }
    say(out, &CliText::Untrusted { id: line(id) })?;
    Ok(0)
}
