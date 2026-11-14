//! Work counters for the linear-time tests. They exist only in test builds.

use std::cell::Cell;

thread_local! {
    /// Bytes examined while looking for a character reference.
    pub(crate) static ENTITY_BYTES: Cell<usize> = const { Cell::new(0) };
    /// Open `!!!` frames touched by the scan.
    pub(crate) static FRAME_OPS: Cell<usize> = const { Cell::new(0) };
    /// Characters stepped by the MDX expression checker.
    pub(crate) static CHECKER_STEPS: Cell<usize> = const { Cell::new(0) };
}

/// Add `n` to a counter.
pub(crate) fn add(counter: &'static std::thread::LocalKey<Cell<usize>>, n: usize) {
    counter.with(|c| c.set(c.get() + n));
}

/// Reset a counter and return what it held.
pub(crate) fn take(counter: &'static std::thread::LocalKey<Cell<usize>>) -> usize {
    counter.with(|c| c.replace(0))
}
