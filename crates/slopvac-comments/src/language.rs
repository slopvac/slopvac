//! The languages 2.x read comments from, and the comment and string syntax of each.

#[cfg(any(
    feature = "lang-c",
    feature = "lang-go",
    feature = "lang-java",
    feature = "lang-proto"
))]
use tree_sitter::Language as Grammar;

/// A source language whose comments can be extracted.
///
/// The set is the 2.x set (`SOURCE_LANGUAGE_BY_EXTENSION` in the Python reference) plus
/// TOML, which 2.x linted for comments only. [`Language::from_name`] takes the names
/// that file discovery assigns to a file kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    /// `python`.
    Python,
    /// `javascript`, including JSX.
    JavaScript,
    /// `typescript`.
    TypeScript,
    /// TypeScript with JSX. Selected by [`Language::refine`] for a `.tsx` file.
    Tsx,
    /// `rust`.
    Rust,
    /// `go`.
    Go,
    /// `java`.
    Java,
    /// `c`.
    C,
    /// `cpp`.
    Cpp,
    /// `csharp`.
    CSharp,
    /// `ruby`.
    Ruby,
    /// `php`.
    Php,
    /// `swift`.
    Swift,
    /// `kotlin`.
    Kotlin,
    /// `scala`.
    Scala,
    /// `shell`.
    Shell,
    /// `lua`.
    Lua,
    /// `perl`.
    Perl,
    /// `r`.
    R,
    /// `elixir`.
    Elixir,
    /// `haskell`.
    Haskell,
    /// `fsharp`.
    FSharp,
    /// `julia`.
    Julia,
    /// `powershell`.
    PowerShell,
    /// `proto`.
    Proto,
    /// `css`.
    Css,
    /// `scss`.
    Scss,
    /// `less`.
    Less,
    /// `clojure`.
    Clojure,
    /// `dart`.
    Dart,
    /// `toml`.
    Toml,
}

const ALL: [Language; 31] = [
    Language::Python,
    Language::JavaScript,
    Language::TypeScript,
    Language::Tsx,
    Language::Rust,
    Language::Go,
    Language::Java,
    Language::C,
    Language::Cpp,
    Language::CSharp,
    Language::Ruby,
    Language::Php,
    Language::Swift,
    Language::Kotlin,
    Language::Scala,
    Language::Shell,
    Language::Lua,
    Language::Perl,
    Language::R,
    Language::Elixir,
    Language::Haskell,
    Language::FSharp,
    Language::Julia,
    Language::PowerShell,
    Language::Proto,
    Language::Css,
    Language::Scss,
    Language::Less,
    Language::Clojure,
    Language::Dart,
    Language::Toml,
];

/// What reads a language's comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// The pure-Rust lexer. It links no C code and is the default.
    Lexer,
    /// A tree-sitter grammar. Only grammars without an external scanner are linked.
    TreeSitter,
}

impl Language {
    /// Every language.
    pub fn all() -> impl Iterator<Item = Self> {
        ALL.into_iter()
    }

    /// The language for a discovery name such as `"python"` or `"csharp"`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::all().find(|language| language.name() == name)
    }

    /// The discovery name. `Tsx` shares `typescript` with [`Language::TypeScript`].
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript | Self::Tsx => "typescript",
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "csharp",
            Self::Ruby => "ruby",
            Self::Php => "php",
            Self::Swift => "swift",
            Self::Kotlin => "kotlin",
            Self::Scala => "scala",
            Self::Shell => "shell",
            Self::Lua => "lua",
            Self::Perl => "perl",
            Self::R => "r",
            Self::Elixir => "elixir",
            Self::Haskell => "haskell",
            Self::FSharp => "fsharp",
            Self::Julia => "julia",
            Self::PowerShell => "powershell",
            Self::Proto => "proto",
            Self::Css => "css",
            Self::Scss => "scss",
            Self::Less => "less",
            Self::Clojure => "clojure",
            Self::Dart => "dart",
            Self::Toml => "toml",
        }
    }

    /// Pick the dialect a file needs. A `.tsx` file selects [`Language::Tsx`]; every
    /// other pair is unchanged.
    #[must_use]
    pub fn refine(self, file_name: &str) -> Self {
        let tsx = file_name
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("tsx"));
        if self == Self::TypeScript && tsx {
            Self::Tsx
        } else {
            self
        }
    }

    /// The backend [`crate::extract()`] uses for this language in this build.
    ///
    /// The lexer reads every language. A tree-sitter grammar replaces it for a language
    /// whose grammar has no external scanner and whose `lang-*` feature is on. Every
    /// other grammar ships C scanner code that reads and writes raw memory, so none is
    /// linked.
    #[must_use]
    pub fn backend(self) -> Backend {
        match self {
            Self::C if cfg!(feature = "lang-c") => Backend::TreeSitter,
            Self::Go if cfg!(feature = "lang-go") => Backend::TreeSitter,
            Self::Java if cfg!(feature = "lang-java") => Backend::TreeSitter,
            Self::Proto if cfg!(feature = "lang-proto") => Backend::TreeSitter,
            _ => Backend::Lexer,
        }
    }

    /// The tree-sitter grammar, when the build links one.
    #[cfg(any(
        feature = "lang-c",
        feature = "lang-go",
        feature = "lang-java",
        feature = "lang-proto"
    ))]
    pub(crate) fn grammar(self) -> Option<Grammar> {
        match self {
            #[cfg(feature = "lang-c")]
            Self::C => Some(tree_sitter_c::LANGUAGE.into()),
            #[cfg(feature = "lang-go")]
            Self::Go => Some(tree_sitter_go::LANGUAGE.into()),
            #[cfg(feature = "lang-java")]
            Self::Java => Some(tree_sitter_java::LANGUAGE.into()),
            #[cfg(feature = "lang-proto")]
            Self::Proto => Some(tree_sitter_proto::LANGUAGE.into()),
            _ => None,
        }
    }

    pub(crate) const fn syntax(self) -> &'static Syntax {
        match self {
            Self::Python => &PYTHON,
            Self::Ruby => &RUBY,
            Self::Shell => &SHELL,
            Self::Perl => &PERL,
            Self::Elixir => &ELIXIR,
            Self::Toml => &TOML,
            Self::R => &R,
            Self::Julia => &JULIA,
            Self::PowerShell => &POWERSHELL,
            Self::Lua => &LUA,
            Self::Haskell => &HASKELL,
            Self::FSharp => &FSHARP,
            Self::Php => &PHP,
            Self::Clojure => &CLOJURE,
            Self::Css => &CSS,
            Self::C | Self::Cpp => &C,
            Self::CSharp => &CSHARP,
            Self::Java => &JAVA,
            Self::Go => &GO,
            Self::Rust => &RUST,
            Self::Swift => &SWIFT,
            Self::Kotlin => &KOTLIN,
            Self::Scala => &SCALA,
            Self::JavaScript | Self::TypeScript | Self::Tsx => &JAVASCRIPT,
            Self::Dart => &DART,
            Self::Proto | Self::Scss | Self::Less => &PROTO,
        }
    }
}

/// How a block comment is delimited.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Block {
    /// Fixed open and close strings.
    Fixed {
        open: &'static str,
        close: &'static str,
        /// A `*` run follows the opener and precedes the closer, and a leading `*`
        /// column decorates the inner lines.
        stars: bool,
        /// The comment nests: each opener inside it needs its own closer.
        nested: bool,
        /// Opener and closer count only at the start of a line (`=begin`, `=end`).
        line_start: bool,
    },
    /// Lua long brackets: `--[[ ]]`, `--[==[ ]==]`.
    Lua,
}

/// A string form. Code inside it is not a comment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Quote {
    pub(crate) delim: &'static str,
    /// The string may run over line ends. Otherwise it ends at the end of its line
    /// unless a backslash escapes the terminator.
    pub(crate) multiline: bool,
    /// `'` opens a string only when a one-character literal follows (`'x'`, `'\n'`).
    /// This keeps Rust lifetimes, C++ digit separators and Haskell primes from
    /// swallowing the rest of the line.
    pub(crate) char_literal: bool,
}

/// The comment and string delimiters of one language.
#[derive(Debug)]
pub(crate) struct Syntax {
    /// Line comment markers, checked in order.
    pub(crate) line: &'static [&'static str],
    /// Markers that start a line comment only at the very start of the file.
    pub(crate) shebang: &'static [&'static str],
    /// Characters that may follow a line marker run and belong to it (`//!`).
    pub(crate) line_suffix: &'static [char],
    /// Line markers (marker run plus suffix) that start a doc comment.
    pub(crate) line_doc: &'static [&'static str],
    /// Characters that open a doc comment when they follow the marker and its blanks
    /// (Haskell's `-- |`). The leader is part of the decoration.
    pub(crate) leaders: &'static [char],
    /// A backslash before the line end continues a line comment onto the next line.
    pub(crate) continuation: bool,
    /// Strings may use Lua long brackets, `[[ ... ]]` and `[==[ ... ]==]`.
    pub(crate) long_strings: bool,
    /// Block forms, checked in order, before line forms.
    pub(crate) blocks: &'static [Block],
    /// String forms, checked in order. List a longer delimiter before its prefix.
    pub(crate) strings: &'static [Quote],
}

const fn quote(delim: &'static str, multiline: bool) -> Quote {
    Quote {
        delim,
        multiline,
        char_literal: false,
    }
}

const DQ: Quote = quote("\"", false);
const SQ: Quote = quote("'", false);
const MDQ: Quote = quote("\"", true);
const MSQ: Quote = quote("'", true);
const BT: Quote = quote("`", true);
const TDQ: Quote = quote("\"\"\"", true);
const TSQ: Quote = quote("'''", true);
const CHAR: Quote = Quote {
    delim: "'",
    multiline: false,
    char_literal: true,
};

const fn star(nested: bool) -> Block {
    Block::Fixed {
        open: "/*",
        close: "*/",
        stars: true,
        nested,
        line_start: false,
    }
}

const fn plain(open: &'static str, close: &'static str, nested: bool) -> Block {
    Block::Fixed {
        open,
        close,
        stars: false,
        nested,
        line_start: false,
    }
}

const SLASH_DOC: &[&str] = &["///", "//!"];
const SHEBANG: &[&str] = &["#!"];

const fn syntax(
    line: &'static [&'static str],
    blocks: &'static [Block],
    strings: &'static [Quote],
) -> Syntax {
    Syntax {
        line,
        shebang: &[],
        line_suffix: &[],
        line_doc: &[],
        leaders: &[],
        continuation: false,
        long_strings: false,
        blocks,
        strings,
    }
}

/// A C-family syntax with `//`, `///`, `//!` and a star block.
const fn c_like(blocks: &'static [Block], strings: &'static [Quote]) -> Syntax {
    Syntax {
        line_suffix: &['!'],
        line_doc: SLASH_DOC,
        ..syntax(&["//"], blocks, strings)
    }
}

const C: Syntax = Syntax {
    continuation: true,
    ..c_like(&[star(false)], &[DQ, CHAR])
};
const CSHARP: Syntax = c_like(&[star(false)], &[TDQ, DQ, CHAR]);
const JAVA: Syntax = c_like(&[star(false)], &[TDQ, DQ, CHAR]);
const GO: Syntax = c_like(&[star(false)], &[BT, DQ, CHAR]);
const RUST: Syntax = Syntax {
    shebang: SHEBANG,
    ..c_like(&[star(true)], &[MDQ, CHAR])
};
const SWIFT: Syntax = Syntax {
    shebang: SHEBANG,
    ..c_like(&[star(true)], &[TDQ, DQ])
};
const KOTLIN: Syntax = Syntax {
    shebang: SHEBANG,
    ..c_like(&[star(true)], &[TDQ, DQ, CHAR])
};
const SCALA: Syntax = Syntax {
    shebang: SHEBANG,
    ..c_like(&[star(true)], &[TDQ, DQ, CHAR])
};
const JAVASCRIPT: Syntax = Syntax {
    shebang: SHEBANG,
    ..c_like(&[star(false)], &[BT, DQ, SQ])
};
const DART: Syntax = Syntax {
    shebang: SHEBANG,
    ..c_like(&[star(false)], &[TDQ, TSQ, DQ, SQ])
};
const PROTO: Syntax = c_like(&[star(false)], &[DQ, SQ]);
const PHP: Syntax = syntax(&["//", "#"], &[star(false)], &[MDQ, MSQ]);
const CSS: Syntax = syntax(&[], &[star(false)], &[DQ, SQ]);

const HASH_SUFFIX: &[char] = &['!'];

const fn hash(blocks: &'static [Block], strings: &'static [Quote]) -> Syntax {
    Syntax {
        line_suffix: HASH_SUFFIX,
        ..syntax(&["#"], blocks, strings)
    }
}

const PYTHON: Syntax = hash(&[], &[TDQ, TSQ, DQ, SQ]);
const SHELL: Syntax = hash(&[], &[MDQ, MSQ, BT]);
const PERL: Syntax = hash(&[], &[MDQ, MSQ]);
const ELIXIR: Syntax = hash(&[], &[TDQ, TSQ, MDQ, MSQ]);
const TOML: Syntax = hash(&[], &[TDQ, TSQ, DQ, SQ]);
const RUBY: Syntax = hash(
    &[Block::Fixed {
        open: "=begin",
        close: "=end",
        stars: false,
        nested: false,
        line_start: true,
    }],
    &[MDQ, MSQ, BT],
);
const R: Syntax = Syntax {
    line_suffix: &['\'', '!'],
    line_doc: &["#'"],
    ..syntax(&["#"], &[], &[MDQ, MSQ])
};
const JULIA: Syntax = hash(&[plain("#=", "=#", true)], &[TDQ, DQ, CHAR]);
const POWERSHELL: Syntax = hash(&[plain("<#", "#>", false)], &[MDQ, MSQ]);
const LUA: Syntax = Syntax {
    long_strings: true,
    ..syntax(&["--"], &[Block::Lua], &[DQ, SQ])
};
const HASKELL: Syntax = Syntax {
    leaders: &['|', '^'],
    ..syntax(&["--"], &[plain("{-", "-}", true)], &[DQ, CHAR])
};
const FSHARP: Syntax = Syntax {
    line_doc: &["///"],
    ..syntax(&["//"], &[plain("(*", "*)", true)], &[TDQ, DQ, CHAR])
};
const CLOJURE: Syntax = syntax(&[";"], &[], &[MDQ]);
