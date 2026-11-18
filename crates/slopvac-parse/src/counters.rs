//! A work counter for tests that bound the cost of mapping projected text to source.
//!
//! The counter exists only with the `test-counters` feature, so ordinary builds carry no
//! shared atomic.

#[cfg(feature = "test-counters")]
static SCAN_WORK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The units of work done so far in this process to map projected text to source: one for
/// each token looked at, and one for each byte of projected text counted to turn a byte
/// offset into a code point offset. A test that lints documents of doubling size and reads
/// the difference shows whether the mapping does work in proportion to the text or to its
/// square.
#[cfg(feature = "test-counters")]
#[doc(hidden)]
#[must_use]
pub fn scan_work() -> usize {
    SCAN_WORK.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(feature = "test-counters")]
pub(crate) fn count_scan(units: usize) {
    SCAN_WORK.fetch_add(units, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(not(feature = "test-counters"))]
pub(crate) fn count_scan(_units: usize) {}
