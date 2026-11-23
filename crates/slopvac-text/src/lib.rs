//! Text analysis shared by the engine: word counting, sentence splitting and the
//! procedural/descriptive classifier. A port of the Python reference
//! (`packages/slopvac-lint/src/slopvac/analyze.py`); offsets are UTF-8 bytes.
//!
//! # Input size
//!
//! The crate does not cap its input. The caller must pass at most
//! 64 MiB (`slopvac_fs::MAX_DOCUMENT_BYTES`) of text, and each slopvac entry point enforces
//! that cap before it calls in. The caps are listed in `docs/contract/policies.md`.
#![allow(
    clippy::allow_attributes_without_reason,
    clippy::expect_used,
    clippy::panic,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]

mod classify;
mod lexicon;
mod measure;
mod messages;
mod pattern;
mod pyrules;
mod rewrite;
mod sentences;
mod tokens;
mod unicode_data;

pub use classify::{TextType, classify_text_type};
pub use measure::{
    bold_spans, clause_boundaries, coordinated_items, dashes_as_aside, is_bold_colon_bullet,
    longest_noun_stack, plain_words, suffix_counts, without_code_and_comments,
};
pub use messages::message_entries;
pub use pattern::{Pattern, PatternError, TextError};
pub use sentences::{Sentence, cell_sentence, split_sentences, split_sentences_within};
#[cfg(feature = "work-counters")]
#[doc(hidden)]
pub use sentences::{take_cuts_found, take_peak_sentences_held, take_sentences_built};
pub use tokens::{count_words, joined_words, tokens};
