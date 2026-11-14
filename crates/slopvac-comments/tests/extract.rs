//! Extraction results per language, worked out by hand in `common`.

mod common;

use common::{FIXTURES, pairs};
use slopvac_comments::CommentKind::{Block, Doc, Line};
use slopvac_comments::{CommentKind, Language};

#[test]
fn every_fixture_extracts_the_expected_comments() {
    let mut wrong = Vec::new();
    for fixture in FIXTURES {
        let got = pairs(fixture.language, fixture.source);
        let want: Vec<(CommentKind, String)> = fixture
            .expected
            .iter()
            .map(|(kind, text)| (*kind, (*text).to_owned()))
            .collect();
        if got != want {
            wrong.push(format!(
                "{}:\n  want {want:?}\n  got  {got:?}",
                fixture.name
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn every_language_has_a_fixture() {
    for language in Language::all() {
        assert!(
            FIXTURES.iter().any(|fixture| fixture.language == language),
            "no fixture for {language:?}"
        );
    }
}

/// The names `FileKind::Source` carries in file discovery, from 2.x's
/// `SOURCE_LANGUAGE_BY_EXTENSION`, plus `toml`.
const DISCOVERY_NAMES: &[&str] = &[
    "python",
    "javascript",
    "typescript",
    "rust",
    "go",
    "java",
    "c",
    "cpp",
    "csharp",
    "ruby",
    "php",
    "swift",
    "kotlin",
    "scala",
    "shell",
    "lua",
    "perl",
    "r",
    "elixir",
    "haskell",
    "fsharp",
    "julia",
    "powershell",
    "proto",
    "css",
    "scss",
    "less",
    "clojure",
    "dart",
    "toml",
];

#[test]
fn every_discovery_name_selects_a_language() {
    for name in DISCOVERY_NAMES {
        let language = Language::from_name(name).unwrap_or_else(|| panic!("no language {name}"));
        assert_eq!(language.name(), *name);
    }
}

#[test]
fn an_unknown_name_selects_no_language() {
    assert_eq!(Language::from_name("cobol"), None);
}

#[test]
fn a_tsx_file_refines_typescript_to_tsx() {
    assert_eq!(Language::TypeScript.refine("App.TSX"), Language::Tsx);
    assert_eq!(Language::TypeScript.refine("app.ts"), Language::TypeScript);
    assert_eq!(Language::Python.refine("x.tsx"), Language::Python);
}

#[test]
fn crlf_line_endings_join_and_split_like_lf() {
    let got = pairs(Language::Rust, "// a\r\n// b\r\n\r\n// c\r\n");
    assert_eq!(got, [(Line, "a b".into()), (Line, "c".into())]);
}

#[test]
fn a_lone_cr_is_a_line_ending() {
    let got = pairs(Language::Python, "# a\r# b\r\r# c\r");
    assert_eq!(got, [(Line, "a b".into()), (Line, "c".into())]);
}

#[test]
fn crlf_inside_a_block_comment_is_a_line_ending() {
    let got = pairs(Language::C, "/*\r\n * one\r\n * two\r\n */\r\n");
    assert_eq!(got, [(Block, "one two".into())]);
}

#[test]
fn an_empty_comment_line_starts_a_new_paragraph() {
    let got = pairs(Language::Rust, "// a\n//\n// b\n");
    assert_eq!(got, [(Line, "a".into()), (Line, "b".into())]);
}

#[test]
fn a_blank_line_between_comments_starts_a_new_fragment() {
    let got = pairs(Language::Go, "// a\n\n// b\n");
    assert_eq!(got, [(Line, "a".into()), (Line, "b".into())]);
}

#[test]
fn inner_and_outer_doc_comments_do_not_join() {
    let got = pairs(Language::Rust, "//! inner\n/// outer\nfn f() {}\n");
    assert_eq!(got, [(Doc, "inner".into()), (Doc, "outer".into())]);
}

#[test]
fn multibyte_text_is_kept_whole() {
    let got = pairs(Language::Rust, "// héllo → wörld 😀\n// ünï\n");
    assert_eq!(got, [(Line, "héllo → wörld 😀 ünï".into())]);
}

#[test]
fn trailing_whitespace_and_a_star_banner_are_decoration() {
    let got = pairs(
        Language::C,
        "/***************\n * Title   \n ***************/\n",
    );
    assert_eq!(got, [(Block, "Title".into())]);
}

#[test]
fn source_without_comments_yields_nothing() {
    let got = pairs(Language::Rust, "fn main() { let x = 1; }\n");
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn an_empty_source_yields_nothing() {
    for language in Language::all() {
        let got = pairs(language, "");
        assert!(got.is_empty(), "{language:?}: {got:?}");
    }
}
