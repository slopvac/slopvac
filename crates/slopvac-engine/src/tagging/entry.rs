//! One block of a document as the taggers read it.
//!
//! The fields that make up a [`BlockView`] are private, so the only way to turn an entry into
//! a block is [`Entry::view`]. The tests count the calls to it to check that a document's
//! blocks are built once, however many taggers ask for them.

use std::ops::Range;

use slopvac_context::{BlockKind as ViewKind, BlockView};
use slopvac_parse::{ByteMapper, Projection};

use std::cell::OnceCell;

pub(super) struct Entry<'a> {
    pub(super) kind: ViewKind,
    pub(super) span: Range<usize>,
    pub(super) text: &'a str,
    heading_path: Vec<&'a str>,
    heading_level: Option<u8>,
    admonition_title: Option<&'a str>,
    pub(super) projection: Option<&'a Projection>,
    /// The index that maps sentence offsets to source, built when a tagger first asks.
    pub(super) mapper: OnceCell<ByteMapper<'a>>,
}

/// What an entry is made of, besides the block it reads.
pub(super) struct Context<'a> {
    pub(super) heading_path: Vec<&'a str>,
    pub(super) heading_level: Option<u8>,
    pub(super) admonition_title: Option<&'a str>,
    pub(super) projection: Option<&'a Projection>,
}

impl<'a> Entry<'a> {
    pub(super) fn new(
        kind: ViewKind,
        span: Range<usize>,
        text: &'a str,
        context: Context<'a>,
    ) -> Self {
        Self {
            kind,
            span,
            text,
            heading_path: context.heading_path,
            heading_level: context.heading_level,
            admonition_title: context.admonition_title,
            projection: context.projection,
            mapper: OnceCell::new(),
        }
    }

    /// The block a tagger reads. Every call builds one.
    pub(super) fn view(&self) -> BlockView<'a> {
        #[cfg(test)]
        counted::built();
        BlockView {
            kind: self.kind,
            span: self.span.clone(),
            text: self.text,
            heading_path: self.heading_path.clone(),
            heading_level: self.heading_level,
            admonition_title: self.admonition_title,
        }
    }
}

/// How many blocks this thread has built, for the tests that bound the work of tagging.
#[cfg(test)]
pub(super) mod counted {
    use std::cell::Cell;

    thread_local! {
        static BUILT: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn built() {
        BUILT.with(|c| c.set(c.get() + 1));
    }

    /// Blocks built on this thread since the last call.
    pub(in crate::tagging) fn take_built() -> usize {
        BUILT.with(|c| c.replace(0))
    }
}
