//! Pack manifests and the namespaced registries that hold what packs provide.
//!
//! [`Manifest::parse`] reads a `slopvac-plugin.toml` under the contract in
//! `docs/contract/plugins.md`, and [`Registry`] keeps named items by qualified
//! name (`<pack>:<name>`) with the [`Provenance`] that supplied each one.

mod core;
mod error;
mod expand;
mod genre_layer;
mod manifest;
mod messages;
mod name;
mod pack;
mod pack_id;
mod problem;
mod provenance;
mod registry;
mod source;
mod synced;

pub use core::core_source;
pub use error::PackError;
pub use messages::message_entries;
pub use pack::{
    LoadedPack, MAX_PACK_BYTES, MAX_PACK_FILES, MAX_RUN_BYTES, MAX_RUN_RULES, Packs,
};
pub use problem::{ArtifactFault, LimitKind, RuleFault, RuleProblem};
pub use synced::{SyncedError, load_synced};
pub use source::{DirSource, EmbeddedSource, FileTable, MANIFEST_FILE, PackSource, SourceError};

pub use manifest::{
    HookDecl, Manifest, ManifestError, PackKind, PackPath, PluginRuntime, PluginTable, Provides,
    Requirement,
};
pub use name::{NameError, QualifiedName};
pub use pack_id::{PackIdError, from_lock_id, to_lock_id};
pub use provenance::{Provenance, Source};
pub use registry::{Entry, Reference, Registry, RegistryError};
