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
pub static ALL: &[Registered] = &[];

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
