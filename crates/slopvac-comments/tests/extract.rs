#![cfg(feature = "all-languages")]
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

/// Source whose first line is a shebang, per grammar family. The grammars emit it as a
/// comment, `hash_bang_line` or `shebang`; 2.x read it as text and so does 3.0.
const SHEBANGS: &[(Language, &str, &str)] = &[
    (
        Language::JavaScript,
        "#!/usr/bin/env node\n// a\nlet x = 1;\n",
        "/usr/bin/env node",
    ),
    (
        Language::TypeScript,
        "#!/usr/bin/env node\n// a\nlet x = 1;\n",
        "/usr/bin/env node",
    ),
    (
        Language::Tsx,
        "#!/usr/bin/env node\n// a\nlet x = 1;\n",
        "/usr/bin/env node",
    ),
    (
        Language::Rust,
        "#!/usr/bin/env run\n// a\nfn main() {}\n",
        "/usr/bin/env run",
    ),
    (
        Language::Kotlin,
        "#!/usr/bin/env kotlin\n// a\nval x = 1\n",
        "/usr/bin/env kotlin",
    ),
    (
        Language::Python,
        "#!/usr/bin/env python\n# a\nx = 1\n",
        "/usr/bin/env python",
    ),
    (
        Language::Ruby,
        "#!/usr/bin/env ruby\n# a\nx = 1\n",
        "/usr/bin/env ruby",
    ),
    (
        Language::Perl,
        "#!/usr/bin/perl\n# a\nmy $x = 1;\n",
        "/usr/bin/perl",
    ),
    (Language::Shell, "#!/bin/sh\n# a\nx=1\n", "/bin/sh"),
    (
        Language::Elixir,
        "#!/usr/bin/env elixir\n# a\nx = 1\n",
        "/usr/bin/env elixir",
    ),
    (
        Language::R,
        "#!/usr/bin/env Rscript\n# a\nx <- 1\n",
        "/usr/bin/env Rscript",
    ),
    (
        Language::Julia,
        "#!/usr/bin/env julia\n# a\nx = 1\n",
        "/usr/bin/env julia",
    ),
    (
        Language::PowerShell,
        "#!/usr/bin/env pwsh\n# a\n$x = 1\n",
        "/usr/bin/env pwsh",
    ),
];

#[test]
fn a_shebang_is_its_own_fragment_in_every_grammar() {
    let mut wrong = Vec::new();
    for (language, source, shebang) in SHEBANGS {
        let got: Vec<String> = pairs(*language, source)
            .into_iter()
            .map(|(_, t)| t)
            .collect();
        if got != [*shebang, "a"] {
            wrong.push(format!("{language:?}: {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_python_docstring_needs_a_plain_or_raw_or_unicode_string() {
    use CommentKind::Docstring;
    for prefix in ["", "r", "R", "u", "U"] {
        let got = pairs(Language::Python, &format!("{prefix}\"\"\"doc\"\"\"\n"));
        assert_eq!(got, [(Docstring, "doc".into())], "prefix {prefix:?}");
    }
    for prefix in ["f", "F", "b", "B", "rb", "bR", "br", "fr", "Rf"] {
        let got = pairs(Language::Python, &format!("{prefix}\"\"\"doc\"\"\"\n"));
        assert!(
            got.is_empty(),
            "prefix {prefix:?} is not a docstring: {got:?}"
        );
    }
}

#[test]
fn a_python_comment_ends_at_a_lone_cr_and_the_code_after_it_is_not_prose() {
    let got = pairs(Language::Python, "# note\rtotal = compute_everything()\r");
    assert_eq!(got, [(Line, "note".into())]);
}

#[test]
fn a_comment_ends_at_a_lone_cr_in_every_hash_and_slash_grammar() {
    let mut wrong = Vec::new();
    for (language, source) in [
        (Language::Ruby, "# note\rtotal = compute_everything\r"),
        (Language::Shell, "# note\rtotal=compute_everything\r"),
        (Language::Toml, "# note\rtotal = 1\r"),
        (Language::Rust, "// note\rfn compute_everything() {}\r"),
        (Language::Go, "// note\rvar compute_everything = 1\r"),
    ] {
        let got: Vec<String> = pairs(language, source)
            .into_iter()
            .map(|(_, t)| t)
            .collect();
        if got != ["note"] {
            wrong.push(format!("{language:?}: {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn elixir_doc_attributes_need_the_at_operator() {
    let got = pairs(
        Language::Elixir,
        "x = !doc(\"a string\")\ny = not moduledoc(\"other\")\n",
    );
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn haskell_comments_are_grouped_like_any_other_line_comments() {
    let trailing = pairs(
        Language::Haskell,
        "x = 1 -- trailing\n-- standalone\ny = 2\n",
    );
    assert_eq!(
        trailing,
        [(Line, "trailing".into()), (Line, "standalone".into())]
    );
    let markers = pairs(Language::Haskell, "-- a\n--- b\ny = 2\n");
    assert_eq!(markers, [(Line, "a".into()), (Line, "b".into())]);
}

#[test]
fn a_bom_does_not_stop_the_first_comments_joining() {
    let got = pairs(Language::Rust, "\u{feff}// a\n// b\n");
    assert_eq!(got, [(Line, "a b".into())]);
}

#[test]
fn every_fixture_gives_the_same_text_in_every_line_ending_and_with_a_bom() {
    let mut wrong = Vec::new();
    for fixture in FIXTURES {
        for (label, source) in common::variants(fixture.source) {
            let got = pairs(fixture.language, &source);
            let want: Vec<(CommentKind, String)> = fixture
                .expected
                .iter()
                .map(|(kind, text)| (*kind, (*text).to_owned()))
                .collect();
            if got != want {
                wrong.push(format!(
                    "{} [{label}]:\n  want {want:?}\n  got  {got:?}",
                    fixture.name
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
