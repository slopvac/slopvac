//! Shared fixtures. Every expected value below was worked out by hand from the source
//! text, not by running the extractor.

#![allow(dead_code, unreachable_pub, missing_docs)]

use slopvac_comments::CommentKind::{Block, Doc, Docstring, Line};
use slopvac_comments::{CommentKind, Fragment, Language, extract};

pub struct Fixture {
    pub name: &'static str,
    pub language: Language,
    pub source: &'static str,
    pub expected: &'static [(CommentKind, &'static str)],
}

/// Extract and flatten to `(kind, text)`.
pub fn pairs(language: Language, source: &str) -> Vec<(CommentKind, String)> {
    fragments(language, source)
        .iter()
        .map(|f| (f.kind(), f.text().to_owned()))
        .collect()
}

pub fn fragments(language: Language, source: &str) -> Vec<Fragment> {
    extract(language, source).expect("grammar loads and parses")
}

pub const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "python comments group and trailing comments stand alone",
        language: Language::Python,
        source: "# one\n# two\n\n# three\nx = 1  # trail\n# after\n",
        expected: &[
            (Line, "one two"),
            (Line, "three"),
            (Line, "trail"),
            (Line, "after"),
        ],
    },
    Fixture {
        name: "python docstrings of module, function and class",
        language: Language::Python,
        source: "\"\"\"Module doc.\n\nSecond para.\n\"\"\"\n\n\ndef f():\n    \"\"\"Doc f.\"\"\"\n    x = \"not a doc\"\n    return x\n\n\nclass A:\n    '''First line\n    continues here.\n    '''\n",
        expected: &[
            (Docstring, "Module doc."),
            (Docstring, "Second para."),
            (Docstring, "Doc f."),
            (Docstring, "First line continues here."),
        ],
    },
    Fixture {
        name: "python string that is not the first statement is code",
        language: Language::Python,
        source: "x = 1\n\"\"\"not first\"\"\"\ny = 'a # not a comment'\n",
        expected: &[],
    },
    Fixture {
        name: "rust doc comments, plain comments, blocks and strings",
        language: Language::Rust,
        source: "//! inner one\n//! inner two\n/// outer\nfn f() {} // trailing\n// plain a\n// plain b\n/* block one\n * block two\n */\n/** doc block\n * more\n */\nfn g() { let s = \"// not\"; }\n",
        expected: &[
            (Doc, "inner one inner two"),
            (Doc, "outer"),
            (Line, "trailing"),
            (Line, "plain a plain b"),
            (Block, "block one block two"),
            (Doc, "doc block more"),
        ],
    },
    Fixture {
        name: "javascript jsdoc and inline block",
        language: Language::JavaScript,
        source: "// line\n/** JSDoc\n * @param x the x\n */\nconst a = '/* no */'; /* inline */\n",
        expected: &[
            (Line, "line"),
            (Doc, "JSDoc @param x the x"),
            (Block, "inline"),
        ],
    },
    Fixture {
        name: "typescript",
        language: Language::TypeScript,
        source: "// a\ninterface A { /** doc */ x: number }\n",
        expected: &[(Line, "a"), (Doc, "doc")],
    },
    Fixture {
        name: "tsx",
        language: Language::Tsx,
        source: "// a\nconst e = <div>{/* jsx */}</div>;\n",
        expected: &[(Line, "a"), (Block, "jsx")],
    },
    Fixture {
        name: "go",
        language: Language::Go,
        source: "// Package p does.\n// Second.\npackage p\n/* block */\nvar s = `// no`\n",
        expected: &[(Line, "Package p does. Second."), (Block, "block")],
    },
    Fixture {
        name: "java",
        language: Language::Java,
        source: "/** Javadoc.\n * @return x\n */\nclass A { // trail\n}\n",
        expected: &[(Doc, "Javadoc. @return x"), (Line, "trail")],
    },
    Fixture {
        name: "c",
        language: Language::C,
        source: "/* a */\n// b\nint x; /* c */\n",
        expected: &[(Block, "a"), (Line, "b"), (Block, "c")],
    },
    Fixture {
        name: "cpp",
        language: Language::Cpp,
        source: "/// doc\n// plain\nint f();\n",
        expected: &[(Doc, "doc"), (Line, "plain")],
    },
    Fixture {
        name: "csharp",
        language: Language::CSharp,
        source: "/// <summary>Sum.</summary>\n// c\nclass A {}\n",
        expected: &[(Doc, "<summary>Sum.</summary>"), (Line, "c")],
    },
    Fixture {
        name: "ruby line and =begin blocks",
        language: Language::Ruby,
        source: "# a\n# b\nx = 1 # c\n=begin\nblock text\nmore\n=end\n",
        expected: &[(Line, "a b"), (Line, "c"), (Block, "block text more")],
    },
    Fixture {
        name: "php keeps // and # apart",
        language: Language::Php,
        source: "<?php\n// a\n# b\n/* c */\n/** d */\n$s = '// no';\n",
        expected: &[(Line, "a"), (Line, "b"), (Block, "c"), (Doc, "d")],
    },
    Fixture {
        name: "swift nested block",
        language: Language::Swift,
        source: "/// doc\n// plain\n/* block /* nested */ still */\nlet s = \"// no\"\n",
        expected: &[
            (Doc, "doc"),
            (Line, "plain"),
            (Block, "block /* nested */ still"),
        ],
    },
    Fixture {
        name: "kotlin",
        language: Language::Kotlin,
        source: "// a\n/** KDoc */\nfun f() {}\n",
        expected: &[(Line, "a"), (Doc, "KDoc")],
    },
    Fixture {
        name: "scala",
        language: Language::Scala,
        source: "// a\n/* b */\nobject O\n",
        expected: &[(Line, "a"), (Block, "b")],
    },
    Fixture {
        name: "shell shebang is its own fragment",
        language: Language::Shell,
        source: "#!/bin/sh\n# a\necho '# no' # c\n",
        expected: &[(Line, "/bin/sh"), (Line, "a"), (Line, "c")],
    },
    Fixture {
        name: "lua line and long brackets",
        language: Language::Lua,
        source: "-- a\n-- b\n--[[ block\nmore ]]\n--[==[ long ]==]\nlocal s = '-- no'\n",
        expected: &[(Line, "a b"), (Block, "block more"), (Block, "long")],
    },
    Fixture {
        name: "perl",
        language: Language::Perl,
        source: "# a\nmy $x = 1; # b\n",
        expected: &[(Line, "a"), (Line, "b")],
    },
    Fixture {
        name: "r roxygen",
        language: Language::R,
        source: "#' Title\n#' more\nf <- function() 1 # b\n",
        expected: &[(Doc, "Title more"), (Line, "b")],
    },
    Fixture {
        name: "elixir comments and doc attributes",
        language: Language::Elixir,
        source: "# a\ndefmodule A do\n  @moduledoc \"\"\"\n  Mod doc.\n  \"\"\"\n  @doc \"one line\"\n  def f, do: 1\nend\n",
        expected: &[
            (Line, "a"),
            (Docstring, "Mod doc."),
            (Docstring, "one line"),
        ],
    },
    Fixture {
        name: "haskell haddock and block",
        language: Language::Haskell,
        source: "-- | haddock\n-- more\nmodule M where\n{- block -}\nx = 1 -- t\n",
        expected: &[(Doc, "haddock more"), (Block, "block"), (Line, "t")],
    },
    Fixture {
        name: "fsharp",
        language: Language::FSharp,
        source: "/// doc\n// plain\n(* block *)\nlet x = 1\n",
        expected: &[(Doc, "doc"), (Line, "plain"), (Block, "block")],
    },
    Fixture {
        name: "julia comments and docstring",
        language: Language::Julia,
        source: "# a\n#= block\nmore =#\n\"\"\"\nDoc f\n\"\"\"\nfunction f() end\n",
        expected: &[(Line, "a"), (Block, "block more"), (Docstring, "Doc f")],
    },
    Fixture {
        name: "powershell",
        language: Language::PowerShell,
        source: "# a\n<# block\nmore #>\n$x = 1 # b\n",
        expected: &[(Line, "a"), (Block, "block more"), (Line, "b")],
    },
    Fixture {
        name: "proto",
        language: Language::Proto,
        source: "// a\n/* b */\nmessage M {}\n",
        expected: &[(Line, "a"), (Block, "b")],
    },
    Fixture {
        name: "css",
        language: Language::Css,
        source: "/* a */\na { color: red; /* b */ }\n",
        expected: &[(Block, "a"), (Block, "b")],
    },
    Fixture {
        name: "scss",
        language: Language::Scss,
        source: "// a\n/* b */\na { b: c }\n",
        expected: &[(Line, "a"), (Block, "b")],
    },
    Fixture {
        name: "less",
        language: Language::Less,
        source: "// a\n/* b */\na { b: c }\n",
        expected: &[(Line, "a"), (Block, "b")],
    },
    Fixture {
        name: "clojure",
        language: Language::Clojure,
        source: "; a\n;; b\n(def x 1) ; c\n",
        expected: &[(Line, "a"), (Line, "b"), (Line, "c")],
    },
    Fixture {
        name: "dart",
        language: Language::Dart,
        source: "/// doc\n/// more\n// plain\nvoid f() {}\n/* b */\n",
        expected: &[(Doc, "doc more"), (Line, "plain"), (Block, "b")],
    },
    Fixture {
        name: "toml",
        language: Language::Toml,
        source: "# a\n# b\nkey = \"# no\" # c\n",
        expected: &[(Line, "a b"), (Line, "c")],
    },
];
