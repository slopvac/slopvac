//! Counters for the matcher work a lint does, for tests that check a rule a document's genre
//! excludes does no work on it.
//!
//! A counter is a matcher entry: one call into the code that looks for a rule's matches in a
//! unit of text or in a diagram. Counters are per thread and per rule kind, so tests that run
//! side by side do not see each other's work. They exist only under the `test-counters`
//! feature. Ordinary builds carry no counter and the call compiles to nothing.

use slopvac_rules::MatchKind;

#[cfg(feature = "test-counters")]
thread_local! {
    static ENTRIES: std::cell::RefCell<std::collections::BTreeMap<MatchKind, usize>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// Record that a matcher of `kind` started on a unit of text or a diagram.
#[cfg(feature = "test-counters")]
pub(crate) fn entered(kind: MatchKind) {
    ENTRIES.with_borrow_mut(|entries| *entries.entry(kind).or_default() += 1);
}

/// Record that a matcher of `kind` started on a unit of text or a diagram.
#[cfg(not(feature = "test-counters"))]
pub(crate) const fn entered(_kind: MatchKind) {}

/// The matcher entries of `kind` made so far on this thread. A test lints one document and
/// reads the difference to see whether a rule of that kind did any work.
#[cfg(feature = "test-counters")]
#[doc(hidden)]
#[must_use]
pub fn matcher_entries(kind: MatchKind) -> usize {
    ENTRIES.with_borrow(|entries| entries.get(&kind).copied().unwrap_or(0))
}
