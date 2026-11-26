//! A document that holds a paragraph of very short sentences cannot make a tagger build a
//! record for each: the default `DocumentView::sentences` stops at a limit that follows the
//! length of the text, and the tagging run ends with the budget error.
#![allow(missing_docs, reason = "test crate: nothing public to document")]

use slopvac_context::{Error, PlainDocument, builtin_registry, take_sentences_built};

#[test]
fn a_paragraph_of_very_short_sentences_ends_the_run_with_the_sentence_limit_error()
-> Result<(), Box<dyn std::error::Error>> {
    // 300000 sentences of 4 bytes. One sentence for every 8 bytes, plus 65,536, allows
    // 150,000 + 65,536 = 215,536 of them.
    let src = format!("{}\n", "Go. ".repeat(300_000));
    let limit = src.len() / 8 + 65_536;
    let doc = PlainDocument::from_markdown("a.md", &src);
    let _ = take_sentences_built();
    let result = builtin_registry()?.tag_document(&doc);
    assert!(
        matches!(result, Err(Error::SentenceLimit { limit: l }) if l == limit),
        "expected the sentence limit error with {limit}"
    );
    let built = take_sentences_built();
    assert!(built <= limit + 1, "{built} records for a limit of {limit}");
    Ok(())
}
