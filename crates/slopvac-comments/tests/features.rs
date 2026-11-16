//! Cargo features pick which languages use a tree-sitter grammar.

use slopvac_comments::{Backend, Language, extract};

#[test]
fn a_language_uses_tree_sitter_only_when_its_feature_is_on() {
    for (language, enabled) in [
        (Language::C, cfg!(feature = "lang-c")),
        (Language::Go, cfg!(feature = "lang-go")),
        (Language::Java, cfg!(feature = "lang-java")),
        (Language::Proto, cfg!(feature = "lang-proto")),
        (Language::Clojure, cfg!(feature = "lang-clojure")),
    ] {
        let want = if enabled {
            Backend::TreeSitter
        } else {
            Backend::Lexer
        };
        assert_eq!(language.backend(), want, "{language:?}");
    }
}

#[test]
fn every_other_language_uses_the_lexer_in_every_build() {
    for language in Language::all() {
        if !matches!(
            language,
            Language::C | Language::Go | Language::Java | Language::Proto | Language::Clojure
        ) {
            assert_eq!(language.backend(), Backend::Lexer, "{language:?}");
        }
    }
}

#[test]
fn every_language_extracts_in_every_build() {
    for language in Language::all() {
        assert!(extract(language, "x\n").is_ok(), "{language:?}");
    }
}
