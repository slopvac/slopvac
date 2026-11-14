//! Plugin discovery, execution and distribution.
//!
//! Modules: the OCI client ([`oci`]), the `.slopvac/packs.lock` model
//! ([`lock`]: strict reader, canonical writer, drift diff and the `outdated`
//! query) and the pack [`index`] reader, builder and search. The plugin host
//! and the trust store land in their own modules.

pub mod index;
pub mod lock;
pub mod oci;
