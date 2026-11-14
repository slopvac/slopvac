//! Text analysis shared by the engine: word counting, sentence splitting and the
//! procedural/descriptive classifier. A port of the Python reference
//! (`packages/slopvac-lint/src/slopvac/analyze.py`); offsets are UTF-8 bytes.

mod classify;
mod lexicon;
mod pattern;
mod sentences;
mod tokens;

pub use classify::{TextType, classify_text_type};
pub use sentences::{Sentence, split_sentences};
pub use tokens::{count_words, tokens};
