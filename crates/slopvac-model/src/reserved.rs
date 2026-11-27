//! The reserved plugin names. This is the one list; every crate that checks a name reads it here.

use schemars::JsonSchema;

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

/// Identifier of a plugin, or of the namespace a publisher signs for. Same character set as
/// `PackId`. `core` is reserved for the built-in plugin that is compiled into the binary, so
/// no manifest, lock entry, index entry or config entry may use it as an id or a namespace.
/// Semantics: docs/contract/plugins.md, section 9.
#[derive(JsonSchema)]
// Schema only. No crate has a Rust type for it yet: the plugin lock and index readers and the
// config validator check the value themselves. The definition lives here so each schema that
// names a plugin id builds it from one place.
#[schemars(with = "String", extend("pattern" = plugin_id_pattern()))]
#[non_exhaustive]
pub struct PluginId;

/// The pattern of a [`PluginId`]: a slug that refuses the reserved id.
#[must_use]
pub fn plugin_id_pattern() -> String {
    format!(r"^(?!{RESERVED_ID}(?![\s\S]))[a-z][a-z0-9-]*(?![\s\S])")
}
