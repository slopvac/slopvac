//! Text analysis shared by the engine: word counting, sentence splitting and the
//! procedural/descriptive classifier. A port of the Python reference
//! (`packages/slopvac-lint/src/slopvac/analyze.py`); offsets are UTF-8 bytes.
//!
//! # Input size
//!
//! The crate does not cap its input. The caller must pass at most
//! 64 MiB (`slopvac_fs::MAX_DOCUMENT_BYTES`) of text, and each slopvac entry point enforces
//! that cap before it calls in. The caps are listed in `docs/contract/policies.md`.

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
pub use sentences::{Sentence, cell_sentence, split_sentences};
pub use tokens::{count_words, joined_words, tokens};
