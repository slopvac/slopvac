//! Load a locked code plugin from its installed tree.
//!
//! `plugins trust` and `plugins status` need the files of a code plugin: the program it runs,
//! its arguments and the declaration that the trust hash covers. The manifest reader lives
//! here, so the helper does too. The binary supplies the parts only it knows: the tree hash
//! that the store verified, the `PATH` of the child and the fingerprint of the signing key.

use std::path::{Path, PathBuf};

use slopvac_plugins::extract::TreeHash;
use slopvac_plugins::host::PluginSpec;
use slopvac_plugins::lock::{LockedPack, PackKind as LockKind, SourceKind};
use slopvac_plugins::protocol::{self, FeatureSet, PluginDeclaration};
use slopvac_plugins::trust::VerifiedPlugin;

use crate::messages::PackMessage;
use crate::{DirSource, Manifest, ManifestError, PackKind, PackPath, PackSource, SourceError};

/// What the caller knows about an installed tree and the child it will start.
#[derive(Debug, Clone)]
pub struct CodePluginContext {
    tree: TreeHash,
    path_env: String,
    key_fingerprint: Option<String>,
}

impl CodePluginContext {
    /// A context for the tree whose hash the store verified, and the `PATH` the child gets.
    #[must_use]
    pub fn new(tree: TreeHash, path_env: impl Into<String>) -> Self {
        Self {
            tree,
            path_env: path_env.into(),
            key_fingerprint: None,
        }
    }

    /// Record the fingerprint of the key that signed the plugin.
    #[must_use]
    pub fn with_key_fingerprint(mut self, fingerprint: String) -> Self {
        self.key_fingerprint = Some(fingerprint);
        self
    }

    /// The hash of the installed tree.
    #[must_use]
    pub const fn tree(&self) -> &TreeHash {
        &self.tree
    }
}

/// Why a code plugin did not load. Every text comes from the message catalog.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CodePluginError {
    /// The lock entry is for a data plugin, which has nothing to run.
    #[error("{}", PackMessage::LoaderNotCode { id: String::from(.id) })]
    NotCode {
        /// The plugin id.
        id: String,
    },
    /// The installed manifest disagrees with the lock entry.
    #[error("{}", PackMessage::LoaderMismatch { id: String::from(.id), field: String::from(*.field) })]
    Mismatch {
        /// The plugin id.
        id: String,
        /// The field that differs: `id`, `version` or `kind`.
        field: &'static str,
    },
    /// The command of the plugin leaves its tree, or an interpreter would read it as an option.
    #[error("{}", PackMessage::LoaderCommandUnsafe { id: String::from(.id) })]
    Command {
        /// The plugin id.
        id: String,
    },
    /// The manifest could not be read.
    #[error("{}", PackMessage::LoaderManifestUnreadable { id: String::from(.id) })]
    Unreadable {
        /// The plugin id.
        id: String,
        /// The read failure.
        #[source]
        source: SourceError,
    },
    /// The manifest breaks the contract.
    #[error("{}", PackMessage::LoaderManifestInvalid { id: String::from(.id) })]
    Manifest {
        /// The plugin id.
        id: String,
        /// What is wrong with the manifest.
        #[source]
        source: ManifestError,
    },
}

/// The verified plugin and the spec to start it, from the lock entry and the installed tree at
/// `root`. The caller has checked `root` against its lock digest.
///
/// The program is the manifest `command` under `root`, the working directory is `root`, and
/// the declaration is read from the same manifest bytes that the tree hash covers. For
/// `runtime = "python"` the caller swaps in the interpreter of the managed environment.
///
/// # Errors
/// [`CodePluginError`] for a data plugin, a manifest that cannot be read or breaks the
/// contract (this includes a `command` that leaves the tree), and a manifest whose id, version
/// or kind differs from the lock entry.
pub fn load_code_plugin(
    locked: &LockedPack,
    root: &Path,
    context: &CodePluginContext,
) -> Result<(VerifiedPlugin, PluginSpec), CodePluginError> {
    let id = locked.id.to_string();
    if locked.kind != LockKind::Code {
        return Err(CodePluginError::NotCode { id });
    }
    let text =
        DirSource::new(root)
            .manifest_text()
            .map_err(|source| CodePluginError::Unreadable {
                id: id.clone(),
                source,
            })?;
    let manifest = Manifest::parse(&text).map_err(|source| CodePluginError::Manifest {
        id: id.clone(),
        source,
    })?;
    let mismatch = |field| CodePluginError::Mismatch {
        id: id.clone(),
        field,
    };
    if manifest.id.as_str() != locked.id.as_str() {
        return Err(mismatch("id"));
    }
    if manifest.version != locked.version {
        return Err(mismatch("version"));
    }
    let (PackKind::Code, Some(table)) = (manifest.kind, manifest.plugin) else {
        return Err(mismatch("kind"));
    };
    // Relative, forward slashes, no `..` segment, and no leading dash: the program is joined
    // to the tree root, and the same text is a script argument for a Python plugin.
    if table.command.starts_with('-') || PackPath::parse(&table.command).is_err() {
        return Err(CodePluginError::Command { id });
    }
    let declaration = PluginDeclaration {
        network: table.network,
        hooks: table
            .hooks
            .iter()
            .map(|h| protocol::HookDecl {
                kind: h.kind.clone(),
                id: h.id.clone(),
            })
            .collect(),
        features: FeatureSet {
            required: table.features_required,
            optional: table.features_optional,
        },
    };
    let spec = PluginSpec {
        name: id.clone(),
        program: root.join(&table.command),
        args: table.args,
        cwd: root.to_path_buf(),
        declaration,
        path_env: context.path_env.clone(),
        env: Vec::new(),
    };
    let from_registry = matches!(locked.source.kind(), SourceKind::Oci(_));
    let plugin = VerifiedPlugin {
        name: id,
        version: Some(locked.version.to_string()),
        source: locked.source.to_string(),
        root: PathBuf::from(root),
        tree: context.tree.clone(),
        manifest_digest: from_registry.then(|| locked.digest.as_str().to_owned()),
        key_fingerprint: context.key_fingerprint.clone(),
        key_id: locked
            .signature
            .as_ref()
            .map(|s| s.key_id.as_str().to_owned()),
    };
    Ok((plugin, spec))
}
