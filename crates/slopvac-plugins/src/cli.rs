//! The `slopvac plugins` command group: its clap tree, the context the binary builds and the
//! subcommands that run on the library.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command};
use slopvac_fs::display_under_root;
use slopvac_messages::{Catalog, render};
use slopvac_sanitize::line;
use slopvac_update::messages::UpdateMessage;

use crate::PluginSpec;
use crate::cause::{self, withheld};
use crate::lock::{
    ConfiguredPack, ConfiguredSource, LockReadError, LockedPack, Lockfile, PackId, PackKind,
    SourceKind, VersionReq, config_drift, entry_drift, outdated as lock_outdated,
    read_project_lock,
};
use crate::messages::cli::{CliText, Redacted, Reported, RunError};
use crate::oci::Digest;
use crate::pyenv::{EnvStore, PythonRuntime};
use crate::revoke::{RevocationError, RevocationNotice, Revocations};
use crate::sign::{DEFAULT_REGISTRY, KeyPolicy, PinScope};
use crate::sync::{
    Change, Network, PackState, PackStatus, PythonBuild, PythonState, Store, SyncError, Syncer,
    status, status_with_python, update_notices,
};
use crate::trust::{
    NamedFile, RotationFiles, TrustError, TrustGate, TrustState, TrustStore, VerifiedPlugin,
};
use crate::update::{LISTING_BUDGET_BYTES, Outcome, Updater, list_candidates};

/// Parse the id of the plugin entry at `position` (counted from 1) in the config.
///
/// # Errors
/// The sentence that names the entry and never the value, when the id is not a valid plugin id.
pub fn config_id(position: usize, text: &str) -> Result<PackId, String> {
    text.parse()
        .map_err(|_| render(&CliText::ConfigIdInvalid { position }))
}

/// Parse the version range that the config sets for plugin `id`.
///
/// # Errors
/// The sentence that names the plugin and never the value, when the range is not valid.
pub fn config_range(id: &str, text: &str) -> Result<VersionReq, String> {
    crate::lock::parse_version_req(text)
        .map_err(|_| render(&CliText::ConfigRangeInvalid { id: line(id) }))
}

/// Parse the source that the config sets for plugin `id`. `root` is the project root and `dir`
/// the directory of the config file that declares the source, both absolute.
///
/// # Errors
/// The sentence that names the plugin and never the value, when the source is not valid.
pub fn config_source(
    id: &str,
    text: &str,
    root: &Path,
    dir: &Path,
) -> Result<ConfiguredSource, String> {
    ConfiguredSource::parse(text, root, dir)
        .map_err(|_| render(&CliText::ConfigSourceInvalid { id: line(id) }))
}

/// Where `slopvac plugins` reads and writes. The caller resolves the directories: the plugin
/// store and the state directory belong to the user, never to the project.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Dirs {
    /// The project root, which holds `.slopvac/plugins.lock`.
    pub project_root: PathBuf,
    /// The plugin store: the per-user data directory that holds installed plugin trees.
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
    /// The verified plugin and the spec it would start with, for the lock entry `locked`.
    ///
    /// The caller read the lock once and passes that entry, so the loader never reads the lock
    /// again: the entry that is loaded is the entry that was checked and the one that is
    /// recorded.
    ///
    /// # Errors
    /// The reason the plugin cannot be loaded, as one sentence with no document text and no
    /// path outside the project, and the stable code of the refusal when it has one.
    fn load(&self, locked: &LockedPack) -> Result<(VerifiedPlugin, PluginSpec), LoadError>;
}

/// Why a plugin could not be loaded. The set is closed and each text comes from the catalog, so
/// no refusal carries a path or document text. A loader keeps its own detail in its log and
/// returns [`LoadError::Refused`], whose detail is withheld.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LoadError {
    /// The cached files of the plugin cannot be read. The code is `plugin-store-unreadable`.
    #[error("{}", render(&CliText::LoadStoreUnreadable { id: id.as_str().to_owned() }))]
    StoreUnreadable {
        /// The plugin id. `plugins trust` names the locked plugin and ignores this value.
        id: PackId,
    },
    /// Any other refusal. Its detail can name a path, so it is not kept.
    #[error("{}", withheld(TRUST_INPUTS))]
    Refused,
}

impl LoadError {
    /// The cached files of plugin `id` cannot be read.
    #[must_use]
    pub fn store_unreadable(id: &PackId) -> Self {
        Self::StoreUnreadable { id: id.clone() }
    }

    /// The stable code of the refusal, when it has one of its own.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::StoreUnreadable { .. } => Some("plugin-store-unreadable"),
            Self::Refused => None,
        }
    }
}

/// A refusal given as text: the text is dropped, because it can name a path.
impl From<String> for LoadError {
    fn from(_: String) -> Self {
        Self::Refused
    }
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
    configured: Vec<ConfiguredPack>,
    core_version: String,
    trusted_env: Vec<Digest>,
    loader: Option<&'a dyn PluginLoader>,
    python: Option<PythonBuild<'a>>,
    envs: Option<&'a EnvStore>,
    executables: BTreeMap<PackId, HashSet<String>>,
    locked: bool,
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
            configured: Vec::new(),
            core_version: String::new(),
            trusted_env: Vec::new(),
            loader: None,
            python: None,
            envs: None,
            executables: BTreeMap::new(),
            locked: false,
        }
    }

    /// The `[[plugins]]` entries of the config, in config order: the one list of configured
    /// plugins. `list` names the ones that are not locked, `outdated` and `update` read their
    /// version requirements (a plugin with none is never updated), and `sync` in CI compares
    /// them with the lock.
    #[must_use]
    pub fn with_configured_packs(mut self, packs: Vec<ConfiguredPack>) -> Self {
        self.configured = packs;
        self
    }

    /// The version requirement of each configured plugin that sets one, by plugin id.
    fn ranges(&self) -> BTreeMap<PackId, VersionReq> {
        self.configured
            .iter()
            .filter_map(|p| Some((p.id.clone(), p.range.clone()?)))
            .collect()
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

    /// The files that become executable in each plugin's tree, by plugin id, as the plugin
    /// manifests declare them. `sync` extracts a native archive plugin with exactly these.
    #[must_use]
    pub fn with_executables(mut self, executables: BTreeMap<PackId, HashSet<String>>) -> Self {
        self.executables = executables;
        self
    }

    /// Run `sync` as CI does: from the lock only. A missing lock file is a failure, because an
    /// empty lock would let the run pass with nothing installed (policies.md, section 1.4).
    #[must_use]
    pub fn with_locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    fn trust_store(&self) -> TrustStore {
        TrustStore::open(&self.dirs.state_dir)
    }

    /// The gate that the host spawns through, built over `vp`, and the spec to hash. A managed
    /// Python plugin runs the interpreter of its environment, so the gate is wired to that
    /// environment and the spec takes the launch the gate gives: the trust hash then matches
    /// what the spawn gate checks.
    ///
    /// # Errors
    /// The reason, as one sentence, when the plugin needs an environment that is not there.
    fn prepared(
        &self,
        vp: &VerifiedPlugin,
        mut spec: PluginSpec,
        code: &'static str,
    ) -> Result<(TrustGate, PluginSpec), String> {
        let mut gate = TrustGate::new(
            self.trust_store(),
            self.trusted_env.clone(),
            vec![vp.clone()],
        )
        .with_revocation_cache(self.revocations.clone());
        // From the manifest bytes the loader's hash pass read, never from the disk again.
        let managed =
            PythonRuntime::from_manifest(&vp.root, &vp.manifest).map_err(|_| withheld(code))?;
        if managed.is_some() {
            let envs = self.envs.ok_or_else(|| render(&CliText::WhyNoEnvs))?;
            gate = gate.with_python_plugins(envs).map_err(|_| withheld(code))?;
            if let Some((program, args)) = gate.python_launch(&vp.name) {
                spec.program = program;
                spec.args = args;
            }
        }
        Ok((gate, spec))
    }
}

fn arg(id: &'static str, help: &CliText) -> Arg {
    Arg::new(id).help(render(help)).required(true)
}

fn registry() -> Arg {
    Arg::new("registry")
        .long("registry")
        .value_name("HOST")
        .default_value(DEFAULT_REGISTRY)
        .value_parser(|v: &str| {
            if crate::oci::valid_registry(v) {
                Ok(v.to_owned())
            } else {
                Err(render(&CliText::BadRegistry))
            }
        })
        .help(render(&CliText::ArgRegistry))
}

/// A namespace: a lowercase letter, then up to 63 lowercase letters, digits or hyphens.
fn namespace_arg() -> Arg {
    arg("namespace", &CliText::ArgNamespace).value_parser(|v: &str| {
        let mut chars = v.chars();
        let ok = v.len() <= 64
            && chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if ok {
            Ok(v.to_owned())
        } else {
            Err(render(&CliText::BadNamespace))
        }
    })
}

/// A key fingerprint: 64 lowercase hex digits (threat-model.md, section 5.2).
fn fingerprint_arg(help: &CliText) -> Arg {
    arg("fingerprint", help).value_parser(|v: &str| {
        if v.len() == 64 && v.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(v.to_owned())
        } else {
            Err(render(&CliText::BadFingerprint))
        }
    })
}

/// A required `--<id> <FILE>` option.
fn file_option(id: &'static str, help: &CliText) -> Arg {
    Arg::new(id)
        .long(id)
        .value_name("FILE")
        .required(true)
        .value_parser(clap::value_parser!(PathBuf))
        .help(render(help))
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
                .arg(namespace_arg())
                .arg(fingerprint_arg(&CliText::ArgFingerprint))
                .arg(file_option("key", &CliText::ArgKey))
                .arg(registry()),
        )
        .subcommand(
            sub("rotate", &CliText::HelpRotate)
                .arg(namespace_arg())
                .arg(fingerprint_arg(&CliText::ArgNewFingerprint))
                .arg(file_option("statement", &CliText::ArgStatement))
                .arg(file_option("old-signature", &CliText::ArgOldSignature))
                .arg(file_option("new-signature", &CliText::ArgNewSignature))
                .arg(registry()),
        )
        .subcommand(sub("new", &CliText::HelpNew).arg(arg("name", &CliText::ArgName)))
        .subcommand(sub("validate", &CliText::HelpValidate).arg(path_arg()))
        .subcommand(sub("test", &CliText::HelpTest).arg(path_arg()))
        .subcommand(sub("build", &CliText::HelpBuild).arg(path_arg()))
        .subcommand(sub("publish", &CliText::HelpPublish).arg(path_arg()))
}

impl RunError {
    /// A failure to set up the group before a subcommand runs, such as a directory that cannot
    /// be resolved. `detail` is one sentence that names a variable or a role and never a value
    /// or a path. It reports `plugins-store-failed`.
    #[must_use]
    pub fn setup_failed(detail: impl Into<String>) -> Self {
        let detail = line(&detail.into());
        Self::Store {
            source: Redacted::new(detail.clone()),
            detail,
        }
    }

    /// A trust store failure while the group is set up, such as reading the pinned keys for
    /// the key policy. It reports the code of the failure, or `plugins-store-failed`, with
    /// every path shown as the commands of the group show it, under `project_root`.
    #[must_use]
    pub fn trust_store(project_root: &Path, e: &TrustError) -> Self {
        store_failed(project_root, e)
    }

    /// The code that the failure reports: the code of its cause when it has one. It is the
    /// same code that a lookup through [`Catalog`] gives.
    #[must_use]
    pub fn code(&self) -> &'static str {
        <Self as Catalog>::code(self)
    }

    /// The exit status of a failed command. Every failure of the group is an incomplete run.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        2
    }
}

/// Checks that need no runner, for the binary to call before it reads any store: `pin` and
/// `rotate` of the reserved namespaces `slopvac` and `core` are refused here, so a damaged trust
/// store cannot hide `namespace-reserved`.
///
/// # Errors
/// [`RunError`] with the code `namespace-reserved`.
pub fn preflight(matches: &ArgMatches) -> Result<(), RunError> {
    if let Some(("pin" | "rotate", m)) = matches.subcommand() {
        let (scope, _) = pin_args(m);
        // The refusal names no path, so no project root is needed to show it.
        crate::trust::reject_reserved(&scope).map_err(|e| store_failed(Path::new(""), &e))?;
    }
    Ok(())
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
        Some(("pin", m)) => run_pin(runner, m, out),
        Some(("rotate", m)) => run_rotate(runner, m, out),
        // STUB(slopvac-8l4.9.62): `search` needs the index fetch, verify, cache and merge.
        // STUB(slopvac-8l4.9.63): the authoring commands.
        Some((name, _)) => Err(RunError::NotWired {
            command: render(&CliText::CommandPath {
                name: name.to_owned(),
            }),
        }),
        None => write(out, command().render_help().to_string().as_bytes()).map(|()| 0),
    }
}

fn write(out: &mut dyn Write, bytes: &[u8]) -> Result<(), RunError> {
    out.write_all(bytes).map_err(|e| RunError::Output {
        detail: e.kind().to_string(),
        source: std::io::Error::from(e.kind()),
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

fn read_lock(runner: &Runner<'_>, required: bool) -> Result<Lockfile, RunError> {
    read_project_lock(&runner.dirs.project_root, required).map_err(LockReadError::into_run_error)
}

fn kind_name(kind: PackKind) -> String {
    kind.as_str().to_owned()
}

/// A trust store failure, with every path shown under `root`, the project root. A failure with
/// a code keeps it.
fn store_failed(root: &Path, e: &TrustError) -> RunError {
    match cause::store(root, e) {
        Ok(source) => RunError::Failed { source },
        Err(detail) => RunError::Store {
            source: Redacted::new(detail.clone()),
            detail,
        },
    }
}

/// A revocation failure, with every path shown under the project root.
fn revocation_failed(e: &RevocationError) -> RunError {
    RunError::Failed {
        source: cause::revocation(e),
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

/// The codes of the messages that carry a withheld sentence in the status and trust commands.
const TRUST_UNKNOWN: &str = "plugins-part-trust-unknown";
const TRUST_INPUTS: &str = "plugins-trust-inputs-failed";

fn trust_part(runner: &Runner<'_>, pack: &LockedPack) -> CliText {
    let id = pack.id.clone();
    let Some(loader) = runner.loader else {
        return CliText::PartTrustUnknown {
            why: render(&CliText::WhyNoLoader),
        };
    };
    let state = loader
        .load(pack)
        .map_err(|_| withheld(TRUST_UNKNOWN))
        .and_then(|(vp, spec)| {
            runner
                .prepared(&vp, spec, TRUST_UNKNOWN)
                .map(|(gate, spec)| (gate, vp, spec))
        })
        .and_then(|(gate, vp, spec)| {
            gate.state(&vp, &spec)
                .map(|(_, s)| s)
                .map_err(|e| store_failed(&runner.dirs.project_root, &e).to_string())
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
    .map_err(|e| RunError::Failed {
        source: cause::sync(&runner.dirs.project_root, &e),
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
    let lock = read_lock(runner, false)?;
    let statuses = statuses(runner, &lock)?;
    let current = runner
        .revocations
        .cached()
        .map_err(|e| revocation_failed(&e))?;
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
    let lock = read_lock(runner, false)?;
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
    for pack in &runner.configured {
        if lock.get(&pack.id).is_none() {
            say(
                out,
                &CliText::ListNotLocked {
                    id: line(pack.id.as_str()),
                },
            )?;
        }
    }
    Ok(0)
}

fn run_sync(runner: &Runner<'_>, out: &mut dyn Write) -> Result<u8, RunError> {
    let lock = read_lock(runner, runner.locked)?;
    if runner.locked {
        // CI runs from the lock, so a lock that the config has outgrown is a failure
        // (policies.md, 1.4): a configured id with no lock entry, a lock entry that nothing
        // configured needs, a configured source that names another source, or a locked
        // version outside the configured requirement. It reads the config and the lock only,
        // before any fetch; the digests are checked by the sync that follows.
        let configured: Vec<PackId> = runner.configured.iter().map(|p| p.id.clone()).collect();
        let mut drift = config_drift(&lock, &configured);
        let entries = entry_drift(&lock, &runner.configured);
        drift.source_differs = entries.source_differs;
        drift.out_of_range = entries.out_of_range;
        if !drift.is_empty() {
            return Err(RunError::Failed {
                source: cause::config_lock_drift(drift),
            });
        }
    }
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
    syncer = syncer.with_executables(runner.executables.clone());
    let report = syncer.sync(&lock).map_err(|e| RunError::Failed {
        source: cause::sync(&runner.dirs.project_root, &e),
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
        if p.repaired {
            say(
                out,
                &CliText::SyncRepaired {
                    id: line(p.id.as_str()),
                },
            )?;
        }
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
    let lock = read_lock(runner, false)?;
    let ranges = runner.ranges();
    let listed = list_candidates(client, &lock, &ranges, |_| true, LISTING_BUDGET_BYTES);
    for line_text in update_notices(&lock, &ranges, &listed.available) {
        write(out, format!("{line_text}\n").as_bytes())?;
    }
    for (id, e) in &listed.results {
        say(
            out,
            &CliText::OutdatedFailed {
                id: line(id.as_str()),
                reason: cause::sync(&runner.dirs.project_root, e).to_string(),
            },
        )?;
    }
    // Keep the pure query in the path, so the lines above and this check agree.
    let _ = lock_outdated(&lock, &ranges, &listed.available);
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
    let ranges = runner.ranges();
    let report = Updater::new(runner.network, runner.policy, runner.revocations)
        .update(&runner.dirs.project_root, &ranges, &ids)
        .map_err(|e| RunError::Failed {
            source: cause::update(&runner.dirs.project_root, &e),
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
                reason: e.code().to_owned(),
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
    let lock = read_lock(runner, false)?;
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
        .ok_or_else(|| refused(render(&CliText::WhyNoLoader)))?;
    let pack_id = id.parse::<PackId>().map_err(|e| refused(e.to_string()))?;
    // One snapshot: this entry is checked against the revocation list and the signature, it is
    // what the loader loads, and its source is what the output reports.
    let pack = lock
        .get(&pack_id)
        .ok_or_else(|| RunError::UnknownPlugin { id: line(id) })?;
    // The locked digest and signing key come first: a path or git source has no manifest
    // digest or key id in the loader's plugin, so the list can name it only through the lock.
    let current = runner
        .revocations
        .cached()
        .map_err(|e| revocation_failed(&e))?;
    notices(out, &current.notices)?;
    if revoked(runner, pack, &current.list) {
        return Err(RunError::Revoked { id: line(id) });
    }
    let (vp, spec) = loader.load(pack).map_err(|e| match e {
        // The sentence names the plugin from the locked entry that was loaded, never an id that
        // the loader supplies.
        LoadError::StoreUnreadable { .. } => RunError::Failed {
            source: Reported::new(
                "plugin-store-unreadable",
                render(&CliText::LoadStoreUnreadable { id: line(id) }),
            ),
        },
        _ => refused(withheld(TRUST_INPUTS)),
    })?;
    // The cache is not trusted (threat model E8), so the signature is checked again here, from
    // the cached files only, before a person's trust is recorded for what they hold.
    let store = Store::new(&runner.dirs.cache_dir);
    let signer = Syncer::new(
        &store,
        Network::Offline,
        runner.policy,
        &runner.dirs.project_root,
        runner.revocations,
    )
    .verify_cached_signature(pack)
    .map_err(|e| {
        let reported = cause::sync(&runner.dirs.project_root, &e);
        match e {
            // A cached file that is not what its name says is an input that `trust` cannot use:
            // the command reports it under its own code, as for a tree that differs.
            SyncError::DigestMismatch { .. } => refused(reported.to_string()),
            _ => RunError::Failed { source: reported },
        }
    })?;
    let (gate, spec) = runner.prepared(&vp, spec, TRUST_INPUTS).map_err(refused)?;
    let (_, state) = gate
        .state(&vp, &spec)
        .map_err(|e| store_failed(&runner.dirs.project_root, &e))?;
    if state == TrustState::Revoked {
        return Err(RunError::Revoked { id: line(id) });
    }
    let record = gate
        .record_trust(&vp, &spec, &runner.now)
        .map_err(|e| store_failed(&runner.dirs.project_root, &e))?;
    say(
        out,
        &CliText::Trusted {
            id: line(id),
            hash: record.hash,
        },
    )?;
    let (kind, source) = provenance(&runner.dirs.project_root, pack);
    say(
        out,
        &match signer {
            Some(key_id) => CliText::TrustedSigned {
                kind,
                origin: source,
                key_id: line(&key_id),
            },
            None => CliText::TrustedUnsigned {
                kind,
                origin: source,
            },
        },
    )?;
    say(out, &CliText::TrustNotContainment)?;
    Ok(0)
}

/// The kind and the redacted source of a lock entry (policies, section 1.7): no user-info, no
/// query or fragment, and a project path shown relative to the project root.
fn provenance(project_root: &Path, pack: &LockedPack) -> (String, String) {
    let kind = pack.source.kind();
    let source = match kind {
        SourceKind::Oci(text) | SourceKind::Archive(text) => crate::oci::redact_url(text),
        SourceKind::Git { url, commit } => format!("{}@{commit}", crate::oci::redact_url(url)),
        SourceKind::Path(rel) => display_under_root(&project_root.join(rel), project_root),
    };
    (kind.name().to_owned(), line(&source))
}

/// The pair of registry host and namespace, and the fingerprint, that `pin` or `rotate` names.
/// The argument parser has checked each one.
fn pin_args(m: &ArgMatches) -> (PinScope, String) {
    let text = |id: &str| m.get_one::<String>(id).cloned().unwrap_or_default();
    (
        PinScope::new(&text("registry"), &text("namespace")),
        text("fingerprint"),
    )
}

fn path_of<'m>(m: &'m ArgMatches, id: &str) -> &'m Path {
    m.get_one::<PathBuf>(id)
        .map_or(Path::new(""), PathBuf::as_path)
}

/// `plugins pin`: pin the key in `--key` for one registry host and namespace. Reads local
/// files only.
fn run_pin(runner: &Runner<'_>, m: &ArgMatches, out: &mut dyn Write) -> Result<u8, RunError> {
    let (scope, fingerprint) = pin_args(m);
    let key = path_of(m, "key");
    let shown = display_under_root(key, &runner.dirs.project_root);
    let pin = runner
        .trust_store()
        .pin_key_file(&scope, &fingerprint, key, &shown, &runner.now)
        .map_err(|e| store_failed(&runner.dirs.project_root, &e))?;
    say(
        out,
        &CliText::Pinned {
            fingerprint: pin.fingerprint,
            namespace: line(scope.namespace()),
            registry: line(scope.registry()),
        },
    )?;
    Ok(0)
}

/// `plugins rotate`: replace a pin with the new key of a rotation statement that the pinned
/// key and the new key both sign. Reads local files and the cached revocation list only.
fn run_rotate(runner: &Runner<'_>, m: &ArgMatches, out: &mut dyn Write) -> Result<u8, RunError> {
    let (scope, fingerprint) = pin_args(m);
    let root = &runner.dirs.project_root;
    let (statement, old_signature, new_signature) = (
        path_of(m, "statement"),
        path_of(m, "old-signature"),
        path_of(m, "new-signature"),
    );
    let shown = [statement, old_signature, new_signature].map(|p| display_under_root(p, root));
    let files = RotationFiles {
        statement: NamedFile {
            path: statement,
            shown: &shown[0],
        },
        old_signature: NamedFile {
            path: old_signature,
            shown: &shown[1],
        },
        new_signature: NamedFile {
            path: new_signature,
            shown: &shown[2],
        },
    };
    let rotated = runner
        .trust_store()
        .rotate_key_files(
            &scope,
            &fingerprint,
            &files,
            runner.revocations,
            &runner.now,
        )
        .map_err(|e| store_failed(&runner.dirs.project_root, &e))?;
    notices(out, &rotated.notices)?;
    say(
        out,
        &CliText::Rotated {
            namespace: line(scope.namespace()),
            registry: line(scope.registry()),
            old: rotated.old_fingerprint,
            new: rotated.pin.fingerprint,
        },
    )?;
    Ok(0)
}

fn run_untrust(runner: &Runner<'_>, id: &str, out: &mut dyn Write) -> Result<u8, RunError> {
    let lock = read_lock(runner, false)?;
    if locked_kind(&lock, id) == Some(PackKind::Data) {
        return Err(RunError::NotCode { id: line(id) });
    }
    let store = runner.trust_store();
    let hashes: Vec<String> = store
        .records()
        .map_err(|e| store_failed(&runner.dirs.project_root, &e))?
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
                &runner.dirs.project_root,
                &TrustError::Corrupt {
                    path: runner.dirs.state_dir.join("trust.json"),
                    why: e.to_string(),
                },
            )
        })?;
        store
            .forget(&digest)
            .map_err(|e| store_failed(&runner.dirs.project_root, &e))?;
    }
    say(out, &CliText::Untrusted { id: line(id) })?;
    Ok(0)
}
