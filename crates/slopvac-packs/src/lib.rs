//! Pack manifests and the namespaced registries that hold what packs provide.
//!
//! [`Manifest::parse`] reads a `slopvac-pack.toml` under the contract in
//! `docs/contract/packs.md`, and [`Registry`] keeps named items by qualified
//! name (`<pack>:<name>`) with the [`Provenance`] that supplied each one.

mod manifest;
mod name;
mod provenance;
mod registry;

pub use manifest::{
    HookDecl, Manifest, ManifestError, PackKind, PackPath, PluginTable, Provides, Requirement,
};
pub use name::{NameError, QualifiedName};
pub use provenance::{Provenance, Source};
pub use registry::{Entry, Registry, RegistryError};
