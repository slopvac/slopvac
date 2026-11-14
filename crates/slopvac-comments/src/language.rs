//! The languages 2.x read comments from, their grammars and their comment delimiters.

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

/// Every language, in declaration order.
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

    /// Pick the grammar variant a file needs. A `.tsx` file selects [`Language::Tsx`];
    /// every other pair is unchanged.
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

    pub(crate) fn grammar(self) -> Grammar {
        match self {
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Self::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Self::Swift => tree_sitter_swift::LANGUAGE.into(),
            Self::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Self::Scala => tree_sitter_scala::LANGUAGE.into(),
            Self::Shell => tree_sitter_bash::LANGUAGE.into(),
            Self::Lua => tree_sitter_lua::LANGUAGE.into(),
            Self::Perl => tree_sitter_perl::LANGUAGE.into(),
            Self::R => tree_sitter_r::LANGUAGE.into(),
            Self::Elixir => tree_sitter_elixir::LANGUAGE.into(),
            Self::Haskell => tree_sitter_haskell::LANGUAGE.into(),
            Self::FSharp => tree_sitter_fsharp::LANGUAGE_FSHARP.into(),
            Self::Julia => tree_sitter_julia::LANGUAGE.into(),
            Self::PowerShell => tree_sitter_powershell::LANGUAGE.into(),
            Self::Proto => tree_sitter_proto::LANGUAGE.into(),
            Self::Css => tree_sitter_css::LANGUAGE.into(),
            Self::Scss => tree_sitter_scss::language(),
            Self::Less => tree_sitter_less::language(),
            Self::Clojure => tree_sitter_clojure_orchard::LANGUAGE.into(),
            Self::Dart => tree_sitter_dart::LANGUAGE.into(),
            Self::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        }
    }

    pub(crate) const fn syntax(self) -> &'static Syntax {
        match self {
            Self::Python | Self::Ruby | Self::Shell | Self::Perl | Self::Elixir | Self::Toml => {
                &HASH
            }
            Self::R => &R,
            Self::Julia => &JULIA,
            Self::PowerShell => &POWERSHELL,
            Self::Lua => &LUA,
            Self::Haskell => &HASKELL,
            Self::FSharp => &FSHARP,
            Self::Php => &PHP,
            Self::Clojure => &SEMICOLON,
            Self::Css => &CSS,
            Self::JavaScript
            | Self::TypeScript
            | Self::Tsx
            | Self::Rust
            | Self::Go
            | Self::Java
            | Self::C
            | Self::Cpp
            | Self::CSharp
            | Self::Swift
            | Self::Kotlin
            | Self::Scala
            | Self::Proto
            | Self::Scss
            | Self::Less
            | Self::Dart => &C_LIKE,
        }
    }
}

/// How a block comment is delimited.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Block {
    /// Fixed open and close strings. `stars` means a `*` run follows the opener and
    /// precedes the closer, and a leading `*` column decorates the inner lines.
    Fixed {
        open: &'static str,
        close: &'static str,
        stars: bool,
    },
    /// Lua long brackets: `--[[ ]]`, `--[==[ ]==]`.
    Lua,
}

/// The comment delimiters of one language family.
#[derive(Debug)]
pub(crate) struct Syntax {
    /// Line comment markers, checked in order.
    pub(crate) line: &'static [&'static str],
    /// Characters that may follow a line marker run and belong to it (`//!`).
    pub(crate) line_suffix: &'static [char],
    /// Line markers (marker run plus suffix) that start a doc comment.
    pub(crate) line_doc: &'static [&'static str],
    /// Characters that open a doc comment when they follow the marker and its blanks
    /// (Haskell's `-- |`). The leader is part of the decoration.
    pub(crate) leaders: &'static [char],
    /// Block forms, checked in order. These win over line forms.
    pub(crate) blocks: &'static [Block],
}

const STAR_BLOCK: Block = Block::Fixed {
    open: "/*",
    close: "*/",
    stars: true,
};

const C_LIKE: Syntax = Syntax {
    line: &["//"],
    line_suffix: &['!'],
    line_doc: &["///", "//!"],
    leaders: &[],
    blocks: &[STAR_BLOCK],
};

const CSS: Syntax = Syntax {
    line: &[],
    line_suffix: &[],
    line_doc: &[],
    leaders: &[],
    blocks: &[STAR_BLOCK],
};

const PHP: Syntax = Syntax {
    line: &["//", "#"],
    line_suffix: &[],
    line_doc: &[],
    leaders: &[],
    blocks: &[STAR_BLOCK],
};

const HASH: Syntax = Syntax {
    line: &["#"],
    line_suffix: &['!'],
    line_doc: &[],
    leaders: &[],
    blocks: &[Block::Fixed {
        open: "=begin",
        close: "=end",
        stars: false,
    }],
};

const R: Syntax = Syntax {
    line: &["#"],
    line_suffix: &['\''],
    line_doc: &["#'"],
    leaders: &[],
    blocks: &[],
};

const JULIA: Syntax = Syntax {
    line: &["#"],
    line_suffix: &[],
    line_doc: &[],
    leaders: &[],
    blocks: &[Block::Fixed {
        open: "#=",
        close: "=#",
        stars: false,
    }],
};

const POWERSHELL: Syntax = Syntax {
    line: &["#"],
    line_suffix: &[],
    line_doc: &[],
    leaders: &[],
    blocks: &[Block::Fixed {
        open: "<#",
        close: "#>",
        stars: false,
    }],
};

const LUA: Syntax = Syntax {
    line: &["--"],
    line_suffix: &[],
    line_doc: &[],
    leaders: &[],
    blocks: &[Block::Lua],
};

const HASKELL: Syntax = Syntax {
    line: &["--"],
    line_suffix: &[],
    line_doc: &[],
    leaders: &['|', '^'],
    blocks: &[Block::Fixed {
        open: "{-",
        close: "-}",
        stars: false,
    }],
};

const FSHARP: Syntax = Syntax {
    line: &["//"],
    line_suffix: &[],
    line_doc: &["///"],
    leaders: &[],
    blocks: &[Block::Fixed {
        open: "(*",
        close: "*)",
        stars: false,
    }],
};

const SEMICOLON: Syntax = Syntax {
    line: &[";"],
    line_suffix: &[],
    line_doc: &[],
    leaders: &[],
    blocks: &[],
};
