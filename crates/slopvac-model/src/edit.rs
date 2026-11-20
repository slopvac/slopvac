use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::schema::x_extensions;
use crate::source::SourceIndex;
use crate::span::{PositionError, Span};

/// Replace the source bytes in span with text, written verbatim. A zero-width span inserts.
/// Edits in one set use original offsets and must not overlap. See
/// docs/contract/positions.md.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
pub struct Edit {
    /// Original-file byte range to replace. A zero-width span inserts.
    pub span: Span,
    /// Replacement text.
    pub text: String,
}

/// Why an edit set cannot apply.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EditError {
    /// An edit span is not valid for the file.
    #[error("{}", crate::messages::ModelText::EditPosition { reason: .0.to_string() })]
    Position(#[from] PositionError),
    /// Two edits overlap.
    #[error("{}", crate::messages::overlap(.first, .second))]
    Overlap {
        /// Earlier edit in sort order.
        first: Span,
        /// Later edit in sort order.
        second: Span,
    },
}

/// Edits in `(start, end)` order, or the first overlapping pair. Touching edits are fine.
pub(crate) fn sorted_without_overlap(edits: &[Edit]) -> Result<Vec<&Edit>, EditError> {
    let mut sorted: Vec<&Edit> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.span.start(), e.span.end()));
    for pair in sorted.windows(2) {
        if pair[1].span.start() < pair[0].span.end() {
            return Err(EditError::Overlap {
                first: pair[0].span,
                second: pair[1].span,
            });
        }
    }
    Ok(sorted)
}

/// Apply an edit set atomically to `text`.
///
/// All spans use original offsets. Edits sort by `(start, end)` and keep array order for
/// equal pairs. Edits that touch do not overlap. An invalid or overlapping set applies
/// nothing.
pub fn apply_edits(text: &str, edits: &[Edit]) -> Result<String, EditError> {
    let index = SourceIndex::disk(text);
    for edit in edits {
        index.check_span(edit.span)?;
    }
    let sorted = sorted_without_overlap(edits)?;
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for edit in sorted {
        out.push_str(&text[cursor..edit.span.start()]);
        out.push_str(&edit.text);
        cursor = edit.span.end();
    }
    out.push_str(&text[cursor..]);
    Ok(out)
}
