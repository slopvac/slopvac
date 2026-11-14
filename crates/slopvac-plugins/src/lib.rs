//! Plugin discovery, execution and distribution.
//!
//! Modules: the OCI client ([`oci`]) and the pack [`index`] reader, builder
//! and search. The plugin host and the trust store land in their own modules.

pub mod index;
pub mod oci;
