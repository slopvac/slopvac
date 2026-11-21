//! The messages `slopvac-packs` reports for synced plugins, as one catalog that the binary
//! registers through [`message_entries`].
//!
//! [`PackMessage`] has one entry per published error code of the loader. Ids in the
//! parameters are validated plugin ids and versions are semver, so no parameter holds text
//! from a plugin file. The ambiguity error is the exception that carries a file name, which
//! the helper escapes before it enters the catalog.

use slopvac_messages::{Catalog, Entry};

use crate::{Provenance, QualifiedName, Reference};

/// Every sentence the loader reports for a synced plugin.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum PackMessage {
    /// A bare name is claimed by more than one plugin.
    #[code("pack-name-ambiguous")]
    #[message("pack-name-ambiguous{at}: `{name}` is ambiguous: {claims}; use a qualified name")]
    NameAmbiguous {
        at: String,
        name: String,
        claims: String,
    },
    /// The config lists a plugin that the lock does not pin.
    #[code("plugin-not-locked")]
    #[message("plugin `{id}` is not in the lock; run `slopvac plugins sync` to pin it")]
    NotLocked { id: String },
    /// The lock pins a plugin that the store does not hold.
    #[code("plugin-not-installed")]
    #[message("plugin `{id}` is locked but not installed; run `slopvac plugins sync`")]
    NotInstalled { id: String },
    /// The stored tree of a plugin differs from its lock digest.
    #[code("plugin-tampered")]
    #[message(
        "the installed tree of plugin `{id}` differs from its lock digest; run `slopvac plugins sync` to restore it"
    )]
    Tampered { id: String },
    /// The store could not be read for a plugin.
    #[code("plugin-store-unreadable")]
    #[message("cannot read the installed tree of plugin `{id}`")]
    StoreUnreadable { id: String },
    /// A plugin targets an extension API that this build does not implement.
    #[code("plugin-api-unsupported")]
    #[message("plugin `{id}` targets extension API {api}, which this slopvac does not implement")]
    ApiUnsupported { id: String, api: String },
    /// The installed manifest disagrees with the lock entry.
    #[code("plugin-manifest-mismatch")]
    #[message("plugin `{id}`: the installed manifest has another {field} than the lock")]
    ManifestMismatch { id: String, field: String },
    /// Plugins require each other in a loop.
    #[code("plugin-requires-cycle")]
    #[message("plugins require each other in a cycle: {cycle}")]
    RequiresCycle { cycle: String },
    /// A required plugin is missing or at a version that does not satisfy the requirement.
    #[code("plugin-requirement-unmet")]
    #[message("plugin `{id}` requires `{needs}` {requirement}, which the lock does not provide")]
    RequirementUnmet {
        id: String,
        needs: String,
        requirement: String,
    },
    /// One plugin is over a size cap.
    #[code("plugin-limit-exceeded")]
    #[message("plugin `{id}` is over the limit of {limit} {what}")]
    PluginLimit {
        id: String,
        what: String,
        limit: String,
    },
    /// The loaded plugins together are over a size cap.
    #[code("plugin-run-limit-exceeded")]
    #[message("the loaded plugins together are over the limit of {limit} {what}")]
    RunLimit { what: String, limit: String },
    /// A plugin directory holds a link, which a plugin may not contain.
    #[code("plugin-file-link")]
    #[message("the plugin holds a link at `{path}`, which is not allowed")]
    FileLink { path: String },
    /// A plugin directory holds more entries than a plugin may.
    #[code("plugin-too-many-entries")]
    #[message("the plugin directory holds more than {limit} entries")]
    TooManyEntries { limit: String },
}

fn claims_text(claims: &[(QualifiedName, Provenance)]) -> String {
    claims
        .iter()
        .map(|(name, p)| format!("`{name}` ({p})"))
        .collect::<Vec<_>>()
        .join(" and ")
}

/// The text of an ambiguous bare name, with `at` pointing at the reference when known.
pub(crate) fn name_ambiguous(
    name: &str,
    claims: &[(QualifiedName, Provenance)],
    at: Option<&Reference>,
) -> String {
    PackMessage::NameAmbiguous {
        at: at.map_or_else(String::new, |r| {
            format!(
                " at {}:{}:{}",
                slopvac_sanitize::line(&r.file),
                r.line,
                r.column
            )
        }),
        name: name.to_owned(),
        claims: claims_text(claims),
    }
    .to_string()
}

/// The entries of the packs catalog.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    PackMessage::entries()
}
