//! The reserved plugin names. This is the one list; every crate that checks a name reads it here.

/// The id and namespace of the built-in plugin. It is reserved: the binary compiles it in and
/// never fetches it, so no lock entry, index entry or config entry may use it, no key, compiled
/// or pinned, verifies a fetched artifact in it, and a reader refuses one that does.
pub const RESERVED_ID: &str = "core";

/// The only first-party namespace. Only a compiled key verifies an artifact in it, and it also
/// names the default index, so no extra index may take it.
pub const FIRST_PARTY_NAMESPACE: &str = "slopvac";

/// Every namespace nobody can pin a key for: the built-in one and the first-party one. Search
/// flags them, `pin_key` refuses them, the trust store never loads a pin for them and no extra
/// index may take them as a name.
pub const RESERVED_NAMESPACES: &[&str] = &[RESERVED_ID, FIRST_PARTY_NAMESPACE];

/// Whether `namespace` is in [`RESERVED_NAMESPACES`]. The match is exact and case-sensitive.
#[must_use]
pub fn is_reserved_namespace(namespace: &str) -> bool {
    RESERVED_NAMESPACES.contains(&namespace)
}
