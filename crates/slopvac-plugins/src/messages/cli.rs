//! Messages of the `slopvac plugins` command group: help text, the status lines and the
//! failures of the group itself.

use slopvac_messages::{Catalog, Entry};

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
    #[text("Show each locked plugin and whether it is installed.")]
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
    #[text("Record trust for the content hash of a code plugin.")]
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
    /// A locked plugin whose tree is installed and intact.
    #[code("plugins-status-installed")]
    #[text("{id} {version}: installed")]
    StatusInstalled { id: String, version: String },
    /// A locked plugin that is not installed.
    #[code("plugins-status-not-installed")]
    #[text("{id} {version}: not installed; run `slopvac plugins sync`")]
    StatusNotInstalled { id: String, version: String },
    /// A locked plugin whose installed tree differs from the lock.
    #[code("plugins-status-changed")]
    #[text("{id} {version}: changed since it was locked; run `slopvac plugins sync`")]
    StatusChanged { id: String, version: String },
    /// Trust for a plugin was removed.
    #[code("plugins-untrusted")]
    #[text("removed the trust recorded for `{id}`")]
    Untrusted { id: String },
}

/// Why a `slopvac plugins` command stopped. Each one exits 2.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub enum RunError {
    /// The subcommand exists in the command tree but has no implementation in this build.
    #[code("plugins-command-unavailable")]
    #[message("`slopvac plugins {command}` is not available in this build yet.")]
    Unavailable {
        /// The subcommand, as the user typed it.
        command: String,
    },
    /// The lock file could not be read or parsed. The detail is the sentence of the failure.
    #[code("plugins-lock-unreadable")]
    #[message("{detail}")]
    LockUnreadable {
        /// What went wrong.
        detail: String,
    },
    /// The store could not be read or written. The detail is the sentence of the failure.
    #[code("plugins-store-failed")]
    #[message("{detail}")]
    StoreFailed {
        /// What went wrong.
        detail: String,
    },
    /// `untrust` named a plugin that has no trust record.
    #[code("plugins-untrust-not-trusted")]
    #[message("no trust is recorded for `{id}`")]
    NotTrusted {
        /// The plugin id.
        id: String,
    },
    /// Writing the output failed.
    #[code("plugins-output-failed")]
    #[message("cannot write the output: {detail}")]
    Output {
        /// What went wrong.
        detail: String,
    },
}

/// The entries of the group's catalogs.
pub(super) fn entries() -> Vec<Entry> {
    [CliText::entries(), RunError::entries()].concat()
}
