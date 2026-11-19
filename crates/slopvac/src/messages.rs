//! The registry of every message catalog in the workspace.
//!
//! Each crate that reports user-facing text derives `Catalog` in its own `messages.rs` and
//! lists it in [`ALL`]. Tests check the list as one registry (unique codes, summaries,
//! placeholders), compare it with `docs/reference/error-codes.lock`, and the reference
//! example writes it to the docs site.

use slopvac_messages::Entry;

/// A catalog's name and a function that returns its entries.
pub type Registered = (&'static str, fn() -> &'static [Entry]);

/// Every catalog in the workspace, by name. Crates add themselves here as they adopt the
/// message registry.
pub static ALL: &[Registered] = &[("report", slopvac_report::message_entries)];

/// A code that no longer exists. Its meaning stays reserved and the reference page keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retired {
    /// The retired code.
    pub code: &'static str,
    /// Why it went away and what replaces it.
    pub note: &'static str,
}

/// Codes removed from the catalogs. Never reuse one.
pub static RETIRED: &[Retired] = &[];

/// The entries of [`ALL`], paired with their catalog names.
#[must_use]
pub fn catalogs() -> Vec<(&'static str, &'static [Entry])> {
    ALL.iter()
        .map(|&(name, entries)| (name, entries()))
        .collect()
}

/// Check the lock file against the registry and the retired list; return one problem per
/// broken rule, empty when all hold.
///
/// The lock has one `code kind` line per code that ever shipped, sorted by code with no
/// duplicates. Every registered code appears with its kind. Every locked code is registered
/// or retired. A retired code stays locked and is not registered again.
#[must_use]
pub fn check_lock(lock: &str, catalogs: &[(&str, &[Entry])], retired: &[Retired]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut locked: Vec<(&str, &str)> = Vec::new();
    for line in lock
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        match line.split_once(' ') {
            Some((code, kind)) => locked.push((code, kind)),
            None => problems.push(format!("lock line {line:?} is not `code kind`")),
        }
    }
    for pair in locked.windows(2) {
        match pair[0].0.cmp(pair[1].0) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => {
                problems.push(format!("`{}` is locked twice", pair[0].0));
            }
            std::cmp::Ordering::Greater => {
                problems.push(format!(
                    "the lock is unsorted: `{}` before `{}`",
                    pair[0].0, pair[1].0
                ));
            }
        }
    }
    let kind_of = |code: &str| locked.iter().find(|(c, _)| *c == code).map(|&(_, k)| k);
    let registered = |code: &str| {
        catalogs
            .iter()
            .flat_map(|(_, entries)| entries.iter())
            .any(|e| e.code == code)
    };
    for entry in catalogs.iter().flat_map(|(_, entries)| entries.iter()) {
        match kind_of(entry.code) {
            None => problems.push(format!(
                "`{}` is not in the lock; add `{} {}`",
                entry.code,
                entry.code,
                entry.kind.name()
            )),
            Some(kind) if kind != entry.kind.name() => problems.push(format!(
                "`{}` is locked as {kind} but registered as {}; a code never changes kind",
                entry.code,
                entry.kind.name()
            )),
            Some(_) => {}
        }
    }
    for &(code, _) in &locked {
        if !registered(code) && !retired.iter().any(|r| r.code == code) {
            problems.push(format!(
                "`{code}` left the catalogs; list it in RETIRED instead of deleting it"
            ));
        }
    }
    for r in retired {
        if kind_of(r.code).is_none() {
            problems.push(format!("retired `{}` is not in the lock", r.code));
        }
        if registered(r.code) {
            problems.push(format!(
                "retired `{}` is registered again; never reuse a code",
                r.code
            ));
        }
    }
    problems
}
