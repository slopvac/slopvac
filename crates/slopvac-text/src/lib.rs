//! Text analysis shared by the engine: word counting, sentence splitting and the
//! procedural/descriptive classifier. A port of the Python reference
//! (`packages/slopvac-lint/src/slopvac/analyze.py`); offsets are UTF-8 bytes.

mod classify;
mod lexicon;
mod measure;
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
pub use pattern::{Pattern, PatternError, TextError};
pub use sentences::{Sentence, split_sentences};
pub use tokens::{count_words, tokens};
