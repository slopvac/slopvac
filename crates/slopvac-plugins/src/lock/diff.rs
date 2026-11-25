use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use semver::Version;

use crate::messages::sync as messages;

use super::model::{Digest, Lockfile, PackId, Source};

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
}

impl LockDiff {
    /// True when the resolved set is exactly the lock.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.unlocked.is_empty()
            && self.changed.is_empty()
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
