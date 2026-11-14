//! Plugin discovery, execution and distribution.
//!
//! Modules: the OCI client ([`oci`]) and the `.slopvac/packs.lock` model
//! ([`lock`]: strict reader, canonical writer, drift diff and the `outdated`
//! query). The plugin host and the trust store land in their own modules.

pub mod lock;
pub mod oci;
