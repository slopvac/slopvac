//! Where a registered item came from.

use std::fmt;

use semver::Version;
use slopvac_model::{PackId, RuleProvenance};

/// The kind of supplier behind a registered item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Source {
    /// The built-in `core` pack.
    Core,
    /// A data pack.
    Pack,
    /// A code plugin.
    Plugin,
}

impl Source {
    /// The lowercase name, as printed in messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Pack => "pack",
            Self::Plugin => "plugin",
        }
    }
}

/// The pack, version, supplier kind and defining file of a registered item.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Provenance {
    /// The pack that supplied the item.
    pub pack: PackId,
    /// The pack version.
    pub version: Version,
    /// The kind of supplier.
    pub source: Source,
    /// Pack-relative path of the defining file, with forward slashes.
    pub file: String,
}

impl Provenance {
    /// Build a provenance record.
    #[must_use]
    pub fn new(pack: PackId, version: Version, source: Source, file: impl Into<String>) -> Self {
        Self {
            pack,
            version,
            source,
            file: file.into(),
        }
    }
}

impl fmt::Display for Provenance {
    /// `pack@version (kind) file`, with the file escaped for one line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@{} ({}) {}",
            self.pack,
            self.version,
            self.source.as_str(),
            slopvac_sanitize::line(&self.file)
        )
    }
}

impl From<&Provenance> for RuleProvenance {
    /// The report shape names the pack, its version and the defining file
    /// (`common.schema.json#/$defs/RuleProvenance`), so `file` becomes `source`.
    fn from(p: &Provenance) -> Self {
        Self {
            pack: p.pack.clone(),
            version: Some(p.version.to_string()),
            source: Some(p.file.clone()),
        }
    }
}
