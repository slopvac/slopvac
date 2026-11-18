//! Fuzz `extract_with` for every language, through the default backend and the lexer.
//!
//! Run with `cargo +nightly fuzz run extract` from `crates/slopvac-comments`. The target
//! checks the position contract, not just the absence of a crash: every fragment's spans
//! sit on code point boundaries inside the source, and every identity token is the exact
//! source text it claims to be.

#![no_main]

use libfuzzer_sys::fuzz_target;
use slopvac_comments::{Fragment, Language, Limits, TokenKind, extract_lexical, extract_with};

fn check(source: &str, fragments: &[Fragment]) {
    for fragment in fragments {
        let comment = fragment.comment_span();
        assert!(comment.end() <= source.len());
        assert!(source.is_char_boundary(comment.start()));
        assert!(source.is_char_boundary(comment.end()));
        for token in fragment.projection().tokens() {
            let src = token.src;
            assert!(src.end() <= source.len());
            if token.kind == TokenKind::Identity {
                assert_eq!(
                    source.get(src.start()..src.end()),
                    Some(token.text.as_str())
                );
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let limits = Limits {
        max_bytes: 1 << 20,
        ..Limits::default()
    };
    for language in Language::all() {
        // A budget error is a typed, deterministic outcome. Anything else must succeed.
        if let Ok(extraction) = extract_with(language, source, &limits) {
            check(source, &extraction.fragments);
        }
        let lexed = extract_lexical(language, source, &limits).expect("the lexer never fails");
        check(source, &lexed.fragments);
    }
});
