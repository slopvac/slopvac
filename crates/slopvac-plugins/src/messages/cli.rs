//! Messages of the `slopvac plugins` command group: help text, the lines the commands print and
//! the failures of the group itself.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use slopvac_messages::{Arg, Catalog, Entry, Kind, Param, ParamKind, Value};
use slopvac_update::messages::UpdateMessage;

use crate::oci::OciError;

/// Help text of the group, its subcommands and their arguments, and the lines the commands
/// print.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum CliText {
    /// Help for the `plugins` group.
    #[code("plugins-help-about")]
    #[text("Manage data plugins and code plugins: sync, trust, update.")]
    HelpAbout,
    /// Help for `plugins list`.
    #[code("plugins-help-list")]
    #[text("List the built-in core and each plugin that the config names.")]
    HelpList,
    /// Help for `plugins search`.
    #[code("plugins-help-search")]
    #[text("Search the plugin indexes that the config names.")]
    HelpSearch,
    /// Help for `plugins sync`.
    #[code("plugins-help-sync")]
    #[text("Install what the lock pins and verify each digest.")]
    HelpSync,
    /// Help for `plugins status`.
    #[code("plugins-help-status")]
    #[text("Show each locked plugin with its kind, install state and trust state.")]
    HelpStatus,
    /// Help for `plugins outdated`.
    #[code("plugins-help-outdated")]
    #[text("List newer versions from the pinned sources.")]
    HelpOutdated,
    /// Help for `plugins update`.
    #[code("plugins-help-update")]
    #[text("Move the lock to the newest versions that the config allows.")]
    HelpUpdate,
    /// Help for `plugins trust`.
    #[code("plugins-help-trust")]
    #[text(
        "Record trust for the content hash of a code plugin. This build cannot load plugin files yet, so it refuses."
    )]
    HelpTrust,
    /// Help for `plugins untrust`.
    #[code("plugins-help-untrust")]
    #[text("Remove the trust recorded for a code plugin.")]
    HelpUntrust,
    /// Help for `plugins pin`.
    #[code("plugins-help-pin")]
    #[text("Pin a publisher key for one namespace on one registry host.")]
    HelpPin,
    /// Help for `plugins rotate`.
    #[code("plugins-help-rotate")]
    #[text("Accept a key rotation for a pinned namespace and replace the pin.")]
    HelpRotate,
    /// Help for `plugins new`.
    #[code("plugins-help-new")]
    #[text("Create the files of a new plugin.")]
    HelpNew,
    /// Help for `plugins validate`.
    #[code("plugins-help-validate")]
    #[text("Check a plugin's files against their schemas.")]
    HelpValidate,
    /// Help for `plugins test`.
    #[code("plugins-help-test")]
    #[text("Run the tests of a plugin.")]
    HelpTest,
    /// Help for `plugins build`.
    #[code("plugins-help-build")]
    #[text("Build the artifact of a plugin.")]
    HelpBuild,
    /// Help for `plugins publish`.
    #[code("plugins-help-publish")]
    #[text("Push the artifact of a plugin to a registry.")]
    HelpPublish,
    /// Help for the search query argument.
    #[code("plugins-help-arg-query")]
    #[text("Words to look for in plugin ids and descriptions.")]
    ArgQuery,
    /// Help for the `--index` option.
    #[code("plugins-help-arg-index")]
    #[text("Search only the configured index with this name.")]
    ArgIndex,
    /// Help for a plugin id argument.
    #[code("plugins-help-arg-id")]
    #[text("The plugin id.")]
    ArgId,
    /// Help for the optional plugin ids of `update`.
    #[code("plugins-help-arg-ids")]
    #[text("Update only these plugins. Default: every locked plugin.")]
    ArgIds,
    /// Help for a namespace argument.
    #[code("plugins-help-arg-namespace")]
    #[text("The publisher namespace.")]
    ArgNamespace,
    /// Help for the fingerprint argument of `pin`.
    #[code("plugins-help-arg-fingerprint")]
    #[text("The key fingerprint: 64 lowercase hex digits.")]
    ArgFingerprint,
    /// Help for the `--registry` option.
    #[code("plugins-help-arg-registry")]
    #[text("The registry host. Default: ghcr.io.")]
    ArgRegistry,
    /// Help for the name argument of `new`.
    #[code("plugins-help-arg-name")]
    #[text("The id of the new plugin.")]
    ArgName,
    /// Help for the plugin directory argument.
    #[code("plugins-help-arg-path")]
    #[text("The plugin directory. Default: the current directory.")]
    ArgPath,
    /// One plugin in `status` or `list`: its id, version, kind and the parts that describe it.
    #[code("plugins-line")]
    #[text("{id} {version} ({kind}): {parts}")]
    Line {
        id: String,
        version: String,
        kind: String,
        parts: String,
    },
    /// The plugin's tree is installed and intact.
    #[code("plugins-part-installed")]
    #[text("installed")]
    PartInstalled,
    /// The plugin's tree is not installed.
    #[code("plugins-part-not-installed")]
    #[text("not installed; run `slopvac plugins sync`")]
    PartNotInstalled,
    /// The installed tree differs from the lock.
    #[code("plugins-part-changed")]
    #[text("changed since it was locked; run `slopvac plugins sync`")]
    PartChanged,
    /// The revocation list names the plugin's digest or signing key.
    #[code("plugins-part-revoked")]
    #[text("revoked")]
    PartRevoked,
    /// The code plugin's content hash has a trust record.
    #[code("plugins-part-trusted")]
    #[text("trusted")]
    PartTrusted,
    /// The code plugin's content hash is named in `SLOPVAC_TRUST`.
    #[code("plugins-part-trusted-env")]
    #[text("trusted by SLOPVAC_TRUST")]
    PartTrustedEnv,
    /// The revocation list names the code plugin's content hash.
    #[code("plugins-part-trust-revoked")]
    #[text("trust revoked")]
    PartTrustRevoked,
    /// The code plugin has no trust record.
    #[code("plugins-part-untrusted")]
    #[text("not trusted; run `slopvac plugins trust {id}`")]
    PartUntrusted { id: String },
    /// The trust state could not be worked out.
    #[code("plugins-part-trust-unknown")]
    #[text("trust state unknown: {why}")]
    PartTrustUnknown { why: String },
    /// The managed Python environment passes its full check.
    #[code("plugins-part-python-verified")]
    #[text("python environment verified")]
    PartPythonVerified,
    /// The managed Python environment is not built.
    #[code("plugins-part-python-missing")]
    #[text("python environment missing; run `slopvac plugins sync`")]
    PartPythonMissing,
    /// The managed Python environment fails its check.
    #[code("plugins-part-python-failed")]
    #[text("python environment failed: {why}")]
    PartPythonFailed { why: String },
    /// The built-in core in `list`.
    #[code("plugins-list-core")]
    #[text("core {version}: built in")]
    ListCore { version: String },
    /// A plugin that the config names and the lock does not.
    #[code("plugins-list-not-locked")]
    #[text("{id}: named in the config, not locked; run `slopvac plugins update`")]
    ListNotLocked { id: String },
    /// One plugin that `sync` handled.
    #[code("plugins-sync-line")]
    #[text("{id} {version}: {change}")]
    SyncLine {
        id: String,
        version: String,
        change: String,
    },
    /// Sync extracted the plugin for the first time.
    #[code("plugins-change-installed")]
    #[text("installed")]
    ChangeInstalled,
    /// Sync found the plugin installed.
    #[code("plugins-change-unchanged")]
    #[text("unchanged")]
    ChangeUnchanged,
    /// Sync replaced a modified tree.
    #[code("plugins-change-reinstalled")]
    #[text("reinstalled")]
    ChangeReinstalled,
    /// Sync used a project directory in place.
    #[code("plugins-change-in-place")]
    #[text("in place")]
    ChangeInPlace,
    /// A notice that does not fail the run.
    #[code("plugins-notice-line")]
    #[text("notice[{code}]: {message}")]
    NoticeLine { code: String, message: String },
    /// `update` moved a plugin to a newer version.
    #[code("plugins-update-updated")]
    #[text("{id}: updated {from} to {to}")]
    UpdateUpdated {
        id: String,
        from: String,
        to: String,
    },
    /// `update` found nothing newer in range.
    #[code("plugins-update-unchanged")]
    #[text("{id}: unchanged at {version}")]
    UpdateUnchanged { id: String, version: String },
    /// `update` found a newer version and refused it.
    #[code("plugins-update-refused")]
    #[text("{id}: refused: {reason}")]
    UpdateRefused { id: String, reason: String },
    /// A newer version exists outside the configured range.
    #[code("plugins-update-outside-range")]
    #[text("{id}: {version} is newer than the range allows")]
    UpdateOutsideRange { id: String, version: String },
    /// A plugin that `outdated` could not check.
    #[code("plugins-outdated-failed")]
    #[text("{id}: cannot check for newer versions: {reason}")]
    OutdatedFailed { id: String, reason: String },
    /// A plugin id in the config that is not a valid id. It names the entry, never the value.
    #[code("plugins-config-id-invalid")]
    #[text("the plugin entry {position} in the config has an id that is not a valid plugin id")]
    ConfigIdInvalid { position: usize },
    /// A version range in the config that does not parse. It names the plugin, never the value.
    #[code("plugins-config-range-invalid")]
    #[text("the version range of plugin `{id}` in the config is not a valid range")]
    ConfigRangeInvalid { id: String },
    /// A lock file that does not parse, with no location.
    #[code("plugins-lock-diagnostic-whole")]
    #[text("not a valid lockfile")]
    LockDiagnosticWhole,
    /// Where a lock file is wrong. The segments are known keys or indexes.
    #[code("plugins-lock-diagnostic-at")]
    #[text("invalid value at {pointer}")]
    LockDiagnosticAt { pointer: String },
    /// Trust was recorded for a code plugin.
    #[code("plugins-trusted")]
    #[text("trusted `{id}` with content hash {hash}")]
    Trusted { id: String, hash: String },
    /// Residual risk R1, said at each `trust`.
    #[code("plugins-trust-not-containment")]
    #[text(
        "Trust is not containment: a trusted plugin runs with your rights, and slopvac has no sandbox."
    )]
    TrustNotContainment,
    /// No loader was given for the plugin files.
    #[code("plugins-why-no-loader")]
    #[text("the plugin files are not loaded")]
    WhyNoLoader,
    /// No store of managed Python environments was given.
    #[code("plugins-why-no-envs")]
    #[text("the managed Python environments are not available")]
    WhyNoEnvs,
    /// Trust for a plugin was removed.
    #[code("plugins-untrusted")]
    #[text("removed the trust recorded for `{id}`")]
    Untrusted { id: String },
    /// The path of a `plugins` subcommand after `slopvac`, for the not-available refusal.
    #[code("plugins-command-path")]
    #[text("plugins {name}")]
    CommandPath { name: String },
}

/// Why a `slopvac plugins` command stopped. Each one exits 2.
///
/// The variants that wrap a library failure keep it as their `source`. A trust store failure
/// is kept as redacted text, because the store's own error names an absolute path.
#[derive(Debug, Catalog)]
#[non_exhaustive]
pub enum RunError {
    /// The subcommand exists in the command tree but has no implementation in this build.
    #[code("command-not-wired")]
    #[message("`slopvac {command}` is not available in this build yet.")]
    NotWired {
        /// The command after `slopvac`, as the user typed it, such as `plugins search`.
        command: String,
    },
    /// The lock file could not be read.
    #[code("plugins-lock-unreadable")]
    #[message("cannot read the lock file: {detail}")]
    LockUnreadable {
        /// What went wrong.
        detail: String,
        /// The read failure, which names no path outside the root.
        #[source]
        source: slopvac_fs::Error,
    },
    /// The lock file is not a valid lock.
    #[code("plugins-lock-invalid")]
    #[message("{detail}")]
    LockInvalid {
        /// The sentence of the failure.
        detail: String,
        /// The same sentence as a cause: the location only, never the text of the lock.
        #[source]
        source: Redacted,
    },
    /// A sync, update, revocation or trust store operation failed. The code and the message are
    /// those of the failure, with every path shown under the project root.
    #[code(delegate)]
    Failed {
        /// The failure as redacted text with its own code.
        #[from]
        source: Reported,
    },
    /// A registry could not be listed.
    #[code("plugins-fetch-failed")]
    #[message("{detail}")]
    Fetch {
        /// The sentence of the failure.
        detail: String,
        /// The registry failure.
        #[source]
        source: Box<OciError>,
    },
    /// The trust store failed, or the group could not be set up. The cause is redacted text: the
    /// store's own error names the absolute path of the file.
    #[code("plugins-store-failed")]
    #[message("{detail}")]
    Store {
        /// The sentence of the failure, with paths shown under the project root.
        detail: String,
        /// The same sentence as a cause, with no further chain.
        #[source]
        source: Redacted,
    },
    /// A command that needs the network ran in offline mode.
    #[code(delegate)]
    Offline {
        /// The shared `offline-network-needed` message.
        #[from]
        source: UpdateMessage,
    },
    /// The id names no plugin in the lock.
    #[code("plugins-unknown-plugin")]
    #[message("`{id}` is not in the lock")]
    UnknownPlugin {
        /// The plugin id.
        id: String,
    },
    /// `trust` or `untrust` named a data plugin.
    #[code("plugin-not-code")]
    #[message("`{id}` is a data plugin; it runs no code, so trust does not apply to it")]
    NotCode {
        /// The plugin id.
        id: String,
    },
    /// `untrust` named a plugin that has no trust record.
    #[code("plugins-untrust-not-trusted")]
    #[message("no trust is recorded for `{id}`")]
    NotTrusted {
        /// The plugin id.
        id: String,
    },
    /// The revocation list names the plugin, so trust is refused.
    #[code("plugins-trust-revoked-plugin")]
    #[message("`{id}` is on the revocation list; trust is refused")]
    Revoked {
        /// The plugin id.
        id: String,
    },
    /// The files of a code plugin could not be loaded for trust.
    #[code("plugins-trust-inputs-failed")]
    #[message("cannot load `{id}` to record trust: {detail}")]
    TrustInputs {
        /// The plugin id.
        id: String,
        /// What went wrong.
        detail: String,
    },
    /// Writing the output failed.
    #[code("plugins-output-failed")]
    #[message("cannot write the output: {detail}")]
    Output {
        /// What went wrong.
        detail: String,
        /// The failure, reduced to its kind: the text carries no path.
        #[source]
        source: std::io::Error,
    },
}

/// A sentence kept as a cause, with no chain behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    text: String,
}

impl Redacted {
    pub(crate) fn new(text: String) -> Self {
        Self { text }
    }
}

impl std::fmt::Display for Redacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for Redacted {}

/// A failure of a library operation, kept as its redacted sentence and the code of that
/// failure. It is a catalog of its own, so a lookup through [`Catalog`] by a generic caller or
/// a trait object reports the code of the failure and not a wrapper's.
///
/// Its entry renders the sentence as `{detail}`. The entries are not in the published catalog: each code
/// is already published by the catalog of the failure it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reported {
    entry: &'static Entry,
    text: String,
}

impl Reported {
    pub(crate) fn new(code: &'static str, text: String) -> Self {
        Self {
            entry: entry_for(code),
            text,
        }
    }
}

/// The entry that renders the sentence under `code`. One is made for each distinct code, once.
fn entry_for(code: &'static str) -> &'static Entry {
    static MADE: Mutex<BTreeMap<&'static str, &'static Entry>> = Mutex::new(BTreeMap::new());
    const PARAMS: &[Param] = &[Param::new("detail", ParamKind::Text)];
    let mut made = MADE.lock().unwrap_or_else(PoisonError::into_inner);
    made.entry(code).or_insert_with(|| {
        // The codes are a closed set of static strings, so the leak is bounded.
        Box::leak(Box::new(Entry::new(
            code,
            Kind::Error,
            "RunError::Failed",
            "{detail}",
            PARAMS,
            "A failed library operation, reported with the code of the failure.",
        )))
    })
}

impl Catalog for Reported {
    fn entries() -> &'static [Entry] {
        &[]
    }

    fn entry(&self) -> &'static Entry {
        self.entry
    }

    fn args(&self) -> Vec<Arg> {
        vec![Arg::new("detail", Value::Text(self.text.clone()))]
    }
}

impl std::fmt::Display for Reported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for Reported {}

/// The entries of the group's catalogs.
pub(super) fn entries() -> Vec<Entry> {
    [
        CliText::entries(),
        RunError::entries(),
        crate::cause::entries(),
    ]
    .concat()
}
