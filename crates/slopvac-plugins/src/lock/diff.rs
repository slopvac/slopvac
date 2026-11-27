use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use semver::{Version, VersionReq};

use crate::messages::sync as messages;

use slopvac_fs::lexical_normalize;

use super::model::{Digest, Lockfile, PackId, Source, SourceKind, ValueError};

/// One pack as the sync code resolved it, before comparing to the lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPack {
    /// Pack id.
    pub id: PackId,
    /// Resolved version.
    pub version: Version,
    /// Resolved source.
    pub source: Source,
    /// Digest of the resolved content.
    pub digest: Digest,
}

/// A value in the lock and the value that was resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delta<T> {
    /// The value in the lock file.
    pub locked: T,
    /// The value that was resolved.
    pub resolved: T,
}

impl<T: PartialEq> Delta<T> {
    fn between(locked: T, resolved: T) -> Option<Self> {
        (locked != resolved).then_some(Self { locked, resolved })
    }
}

/// A pack that is in both sets but differs. At least one field is `Some`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Pack id.
    pub id: PackId,
    /// Set when the version differs.
    pub version: Option<Delta<Version>>,
    /// Set when the source differs.
    pub source: Option<Delta<Source>>,
    /// Set when the digest differs.
    pub digest: Option<Delta<Digest>>,
}

/// How a resolved set differs from the lock. All lists are sorted by id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LockDiff {
    /// Resolved but not locked.
    pub added: Vec<ResolvedPack>,
    /// Locked but not resolved.
    pub removed: Vec<PackId>,
    /// Configured but not locked. Only [`config_drift`] fills it: the id is all that is known
    /// before the pack is resolved.
    pub unlocked: Vec<PackId>,
    /// In both, with a different version, source or digest.
    pub changed: Vec<Change>,
    /// Configured with a source that names a different source from the lock entry. Only
    /// [`entry_drift`] fills it. It keeps the id alone, because a configured source can carry a
    /// credential and must not reach a message.
    pub source_differs: Vec<PackId>,
    /// Locked at a version that the configured requirement does not allow. Only
    /// [`entry_drift`] fills it.
    pub out_of_range: Vec<OutOfRange>,
}

/// A configured pack whose locked version is outside its configured requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutOfRange {
    /// Pack id.
    pub id: PackId,
    /// The version in the lock file.
    pub locked: Version,
    /// The requirement in the config.
    pub range: VersionReq,
}

impl OutOfRange {
    /// `id` is locked at `locked`, which `range` does not allow.
    #[must_use]
    pub fn new(id: PackId, locked: Version, range: VersionReq) -> Self {
        Self { id, locked, range }
    }
}

/// The `source` of a `[[plugins]]` entry in the config, as the drift check compares it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfiguredSource {
    /// A source of a form the lock accepts. A path is relative to the project root.
    Named(Source),
    /// A path that resolves outside the project root. No lock entry can hold it, so it never
    /// matches.
    OutsideProject,
}

impl ConfiguredSource {
    /// Parses the configured source `text` with the lock's own source parser. `root` is the
    /// project root, and `dir` the directory of the config file that declares the source. A
    /// path is resolved against `dir` and made relative to `root` first, because a config path
    /// is relative to its own file and the lock's path to the root. Both are absolute; the
    /// resolution is lexical and reads nothing.
    ///
    /// # Errors
    /// [`ValueError`] when `text` is not an `oci://` reference, an `https://` archive URL, a
    /// `git+https://` URL pinned to a commit or a relative path. The message never repeats the
    /// text, which can carry a credential.
    pub fn parse(text: &str, root: &Path, dir: &Path) -> Result<Self, ValueError> {
        let parsed = text.parse::<Source>();
        if let Ok(source) = &parsed
            && !matches!(source.kind(), SourceKind::Path(_))
        {
            return Ok(Self::Named(source.clone()));
        }
        // The lock refuses `..`, which a config path may use to reach the root from its own
        // file. Every other rule of the lock applies to the text as written.
        let probe = text
            .split('/')
            .map(|segment| if segment == ".." { "_" } else { segment })
            .collect::<Vec<_>>()
            .join("/");
        match (parsed, probe.parse::<Source>()) {
            (Ok(_), _) | (Err(_), Ok(_)) => Ok(Self::resolve(text, root, dir)),
            (Err(e), Err(_)) => Err(e),
        }
    }

    /// The path `text`, which the lock's rules accept but for `..`, resolved against `dir` and
    /// written relative to `root` with `/` between segments.
    fn resolve(text: &str, root: &Path, dir: &Path) -> Self {
        let resolved = lexical_normalize(&dir.join(text));
        let Ok(relative) = resolved.strip_prefix(lexical_normalize(root)) else {
            return Self::OutsideProject;
        };
        let segments: Option<Vec<&str>> = relative.iter().map(|s| s.to_str()).collect();
        segments
            .and_then(|s| s.join("/").parse::<Source>().ok())
            .map_or(Self::OutsideProject, Self::Named)
    }

    /// True when this names the source `locked` holds. An `oci://` source compares its
    /// registry and repository only, because the locked version and digest pin the content.
    fn names(&self, locked: &Source) -> bool {
        let Self::Named(source) = self else {
            return false;
        };
        match (source.kind(), locked.kind()) {
            (SourceKind::Oci(a), SourceKind::Oci(b)) => oci_repository(a) == oci_repository(b),
            _ => source == locked,
        }
    }
}

/// The registry and repository of an `oci://` reference: the text before the first `@`, less a
/// `:<tag>` whose `:` comes after the last `/`, so a registry port stays.
fn oci_repository(text: &str) -> &str {
    let name = text.split_once('@').map_or(text, |(name, _)| name);
    match name.rsplit_once(':') {
        Some((repository, tag)) if !tag.contains('/') => repository,
        _ => name,
    }
}

/// One `[[plugins]]` entry of the config, as the drift check reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConfiguredPack {
    /// Pack id.
    pub id: PackId,
    /// The configured source.
    pub source: ConfiguredSource,
    /// The configured version requirement, when the entry sets one.
    pub range: Option<VersionReq>,
}

impl ConfiguredPack {
    /// An entry for `id` with its source and optional version requirement.
    #[must_use]
    pub fn new(id: PackId, source: ConfiguredSource, range: Option<VersionReq>) -> Self {
        Self { id, source, range }
    }
}

impl LockDiff {
    /// True when the resolved set is exactly the lock.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.unlocked.is_empty()
            && self.changed.is_empty()
            && self.source_differs.is_empty()
            && self.out_of_range.is_empty()
    }
}

impl fmt::Display for LockDiff {
    /// One line per difference, for the CI message (exit 2).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for pack in &self.added {
            writeln!(f, "{}", messages::diff_added(&pack.id, &pack.version))?;
        }
        for id in &self.removed {
            writeln!(f, "{}", messages::diff_removed(id))?;
        }
        for id in &self.unlocked {
            writeln!(f, "{}", messages::diff_unlocked(id))?;
        }
        for change in &self.changed {
            write!(f, "{}", messages::diff_changed(&change.id))?;
            if let Some(d) = &change.version {
                write!(f, "{}", messages::diff_version(&d.locked, &d.resolved))?;
            }
            if let Some(d) = &change.source {
                // A source is free text; ids, versions and digests are validated.
                write!(
                    f,
                    "{}",
                    messages::diff_source(&d.locked.to_string(), &d.resolved.to_string())
                )?;
            }
            if let Some(d) = &change.digest {
                write!(f, "{}", messages::diff_digest(&d.locked, &d.resolved))?;
            }
            writeln!(f)?;
        }
        for id in &self.source_differs {
            writeln!(f, "{}", messages::diff_config_source(id))?;
        }
        for pack in &self.out_of_range {
            writeln!(
                f,
                "{}",
                messages::diff_out_of_range(&pack.id, &pack.locked, &pack.range)
            )?;
        }
        Ok(())
    }
}

/// Compares the resolved set with the lock. When `resolved` repeats an id, the
/// last entry wins.
#[must_use]
pub fn diff(lock: &Lockfile, resolved: &[ResolvedPack]) -> LockDiff {
    let resolved: BTreeMap<&PackId, &ResolvedPack> = resolved.iter().map(|r| (&r.id, r)).collect();
    let mut out = LockDiff::default();
    for (id, r) in &resolved {
        match lock.get(id) {
            None => out.added.push((*r).clone()),
            Some(l) => {
                let version = Delta::between(l.version.clone(), r.version.clone());
                let source = Delta::between(l.source.clone(), r.source.clone());
                let digest = Delta::between(l.digest.clone(), r.digest.clone());
                if version.is_some() || source.is_some() || digest.is_some() {
                    out.changed.push(Change {
                        id: (*id).clone(),
                        version,
                        source,
                        digest,
                    });
                }
            }
        }
    }
    out.removed = lock
        .packs()
        .iter()
        .filter(|l| !resolved.contains_key(&l.id))
        .map(|l| l.id.clone())
        .collect();
    out
}

/// The drift that needs no resolver: a configured pack with no lock entry, and a locked pack
/// that no configured pack needs any more (directly or through `requires`). It checks ids
/// only; versions, sources and digests need the resolved set and [`diff()`].
#[must_use]
pub fn config_drift(lock: &Lockfile, configured: &[PackId]) -> LockDiff {
    let mut needed: BTreeSet<&PackId> = BTreeSet::new();
    let mut pending: Vec<&PackId> = configured.iter().collect();
    while let Some(id) = pending.pop() {
        if needed.insert(id)
            && let Some(locked) = lock.get(id)
        {
            pending.extend(locked.requires.iter());
        }
    }
    LockDiff {
        unlocked: configured
            .iter()
            .filter(|id| lock.get(id).is_none())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        removed: lock
            .packs()
            .iter()
            .filter(|l| !needed.contains(&l.id))
            .map(|l| l.id.clone())
            .collect(),
        ..LockDiff::default()
    }
}

/// The drift between the configured `[[plugins]]` entries and the lock that [`config_drift`]
/// leaves out: a configured source that names a different source from its lock entry, and a
/// locked version outside the configured requirement. An entry with no lock entry is
/// [`config_drift`]'s `unlocked` case, so it is skipped here. Pure: it reads only its inputs.
#[must_use]
pub fn entry_drift(lock: &Lockfile, configured: &[ConfiguredPack]) -> LockDiff {
    let mut out = LockDiff::default();
    for entry in configured {
        let Some(locked) = lock.get(&entry.id) else {
            continue;
        };
        if !entry.source.names(&locked.source) {
            out.source_differs.push(entry.id.clone());
        }
        if let Some(range) = &entry.range
            && !range.matches(&locked.version)
        {
            out.out_of_range.push(OutOfRange::new(
                entry.id.clone(),
                locked.version.clone(),
                range.clone(),
            ));
        }
    }
    out.source_differs.sort();
    out.out_of_range.sort_by(|a, b| a.id.cmp(&b.id));
    out
}
