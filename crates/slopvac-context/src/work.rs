//! Work counters for the tests that bound the cost of tagging.
//!
//! Each counter is per thread, so tests that run in parallel do not see one another. The
//! counters exist only with the `work-counters` feature. Without it, the update functions
//! are empty and the compiler removes the calls.

#[cfg(feature = "work-counters")]
mod enabled {
    use std::cell::Cell;

    thread_local! {
        static SECTION: Cell<usize> = const { Cell::new(0) };
        static TEXT: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn section(n: usize) {
        SECTION.with(|c| c.set(c.get() + n));
    }

    pub(crate) fn text(n: usize) {
        TEXT.with(|c| c.set(c.get() + n));
    }

    /// Blocks examined to find section bounds since the last call.
    #[must_use]
    pub fn take_section_steps() -> usize {
        SECTION.with(|c| c.replace(0))
    }

    /// Positions examined while cleaning sentence text since the last call.
    #[must_use]
    pub fn take_text_steps() -> usize {
        TEXT.with(|c| c.replace(0))
    }
}

#[cfg(feature = "work-counters")]
pub(crate) use enabled::{section, text};
#[cfg(feature = "work-counters")]
pub use enabled::{take_section_steps, take_text_steps};

#[cfg(not(feature = "work-counters"))]
#[inline(always)]
pub(crate) fn section(_n: usize) {}

#[cfg(not(feature = "work-counters"))]
#[inline(always)]
pub(crate) fn text(_n: usize) {}
