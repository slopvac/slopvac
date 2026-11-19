//! Bounds on what one extraction retains.
//!
//! A logical location repeats an identifier in every fragment. Without a cap, one long
//! identifier shared by many members costs identifier length times member count. The
//! identifier is clipped, and the fragment count and retained bytes are capped, so the
//! memory of one extraction grows linearly with its input.

use std::borrow::Cow;

use crate::{BudgetHit, ExtractionStatus, Fragment};

/// Most characters of an identifier or page name kept in a logical location.
pub const MAX_IDENT_CHARS: usize = 128;
/// Most fragments one extraction keeps.
pub const MAX_FRAGMENTS: usize = 100_000;
/// Most bytes of `logical` plus `text` one extraction keeps, summed over its fragments.
pub const MAX_RETAINED_BYTES: usize = 16 * 1024 * 1024;

/// Cap name: the fragment count.
pub(crate) const CAP_FRAGMENTS: &str = "fragments";
/// Cap name: the retained bytes of `logical` plus `text`.
pub(crate) const CAP_RETAINED_BYTES: &str = "retained-bytes";
/// Cap name: the inflated bytes of compressed draw.io pages.
pub(crate) const CAP_INFLATED_BYTES: &str = "inflated-bytes";
/// Cap name: the XML nodes of one draw.io file.
pub(crate) const CAP_XML_NODES: &str = "xml-nodes";

/// Clip an identifier to [`MAX_IDENT_CHARS`] characters, ending in `…` when it was cut.
///
/// Clipping is idempotent, so an identifier clipped once stays valid for later pushes.
pub(crate) fn clip(ident: &str) -> Cow<'_, str> {
    match ident.char_indices().nth(MAX_IDENT_CHARS) {
        Some((end, _)) => Cow::Owned(format!("{}…", &ident[..end])),
        None => Cow::Borrowed(ident),
    }
}

/// The status for a sink that did or did not drop fragments.
pub(crate) fn status(hit: Option<BudgetHit>) -> ExtractionStatus {
    hit.map_or(ExtractionStatus::Ok, ExtractionStatus::BudgetExhausted)
}

/// Collects fragments up to the caps. Once a cap is hit it drops every later fragment.
#[derive(Default)]
pub(crate) struct Sink {
    fragments: Vec<Fragment>,
    retained: usize,
    hit: Option<BudgetHit>,
}

impl Sink {
    /// Keep `fragment` unless a cap is reached. Return whether it was kept.
    pub(crate) fn push(&mut self, fragment: Fragment) -> bool {
        if self.hit.is_some() {
            return false;
        }
        let cost = fragment.logical.len() + fragment.text.as_ref().map_or(0, String::len);
        if self.fragments.len() >= MAX_FRAGMENTS {
            self.hit = Some(BudgetHit::new(CAP_FRAGMENTS, MAX_FRAGMENTS as u64));
            return false;
        }
        if self.retained.saturating_add(cost) > MAX_RETAINED_BYTES {
            self.hit = Some(BudgetHit::new(
                CAP_RETAINED_BYTES,
                MAX_RETAINED_BYTES as u64,
            ));
            return false;
        }
        self.retained += cost;
        self.fragments.push(fragment);
        true
    }

    /// Whether a cap has been hit. Extractors stop reading when it is.
    pub(crate) fn is_full(&self) -> bool {
        self.hit.is_some()
    }

    /// The kept fragments, and the cap that dropped some, if any.
    pub(crate) fn finish(self) -> (Vec<Fragment>, Option<BudgetHit>) {
        (self.fragments, self.hit)
    }
}
