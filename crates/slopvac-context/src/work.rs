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
        static HEADING: Cell<usize> = const { Cell::new(0) };
        static ADMONITION: Cell<usize> = const { Cell::new(0) };
        static LOOKUP: Cell<usize> = const { Cell::new(0) };
        static BUILD: Cell<usize> = const { Cell::new(0) };
        static MATERIALISED: Cell<usize> = const { Cell::new(0) };
        static NEGATION: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn negation(n: usize) {
        NEGATION.with(|c| c.set(c.get() + n));
    }

    /// Bytes of text before an enforcement verb examined to decide whether it is negated,
    /// since the last call.
    #[must_use]
    pub fn take_negation_steps() -> usize {
        NEGATION.with(|c| c.replace(0))
    }

    pub(crate) fn materialised(n: usize) {
        MATERIALISED.with(|c| c.set(c.get() + n));
    }

    /// Tags and pieces of tags built through a tag sink since the last call.
    #[must_use]
    pub fn take_materialised_tags() -> usize {
        MATERIALISED.with(|c| c.replace(0))
    }

    pub(crate) fn build(n: usize) {
        BUILD.with(|c| c.set(c.get() + n));
    }

    /// Tags handed to the index structures since the last call: each structure built counts
    /// the tags it was built from.
    #[must_use]
    pub fn take_build_elements() -> usize {
        BUILD.with(|c| c.replace(0))
    }

    pub(crate) fn section(n: usize) {
        SECTION.with(|c| c.set(c.get() + n));
    }

    pub(crate) fn text(n: usize) {
        TEXT.with(|c| c.set(c.get() + n));
    }

    pub(crate) fn heading(n: usize) {
        HEADING.with(|c| c.set(c.get() + n));
    }

    pub(crate) fn admonition(n: usize) {
        ADMONITION.with(|c| c.set(c.get() + n));
    }

    pub(crate) fn lookup(n: usize) {
        LOOKUP.with(|c| c.set(c.get() + n));
    }

    /// Index steps taken to answer context lookups since the last call: binary-search
    /// probes and tree nodes visited.
    #[must_use]
    pub fn take_lookup_steps() -> usize {
        LOOKUP.with(|c| c.replace(0))
    }

    /// Bytes of admonition title matched against an admonition pattern since the last call.
    #[must_use]
    pub fn take_admonition_steps() -> usize {
        ADMONITION.with(|c| c.replace(0))
    }

    /// Bytes of heading text matched against a heading pattern since the last call.
    #[must_use]
    pub fn take_heading_steps() -> usize {
        HEADING.with(|c| c.replace(0))
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
pub(crate) use enabled::{
    admonition, build, heading, lookup, materialised, negation, section, text,
};
#[cfg(feature = "work-counters")]
pub use enabled::{
    take_admonition_steps, take_build_elements, take_heading_steps, take_lookup_steps,
    take_materialised_tags, take_negation_steps, take_section_steps, take_text_steps,
};

#[cfg(not(feature = "work-counters"))]
#[inline(always)]
pub(crate) fn section(_n: usize) {}

#[cfg(not(feature = "work-counters"))]
#[inline(always)]
pub(crate) fn text(_n: usize) {}

#[cfg(not(feature = "work-counters"))]
#[inline(always)]
pub(crate) fn heading(_n: usize) {}

#[cfg(not(feature = "work-counters"))]
#[inline(always)]
pub(crate) fn admonition(_n: usize) {}

#[cfg(not(feature = "work-counters"))]
#[inline(always)]
pub(crate) fn lookup(_n: usize) {}

#[cfg(not(feature = "work-counters"))]
#[inline(always)]
pub(crate) fn build(_n: usize) {}

#[cfg(not(feature = "work-counters"))]
#[inline(always)]
pub(crate) fn materialised(_n: usize) {}

#[cfg(not(feature = "work-counters"))]
#[inline(always)]
pub(crate) fn negation(_n: usize) {}
