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

    /// The cargo feature that builds this language's grammar, such as `lang-python`.
    #[must_use]
    pub const fn feature(self) -> &'static str {
        match self {
            Self::Python => "lang-python",
            Self::JavaScript => "lang-javascript",
            Self::TypeScript | Self::Tsx => "lang-typescript",
            Self::Rust => "lang-rust",
            Self::Go => "lang-go",
            Self::Java => "lang-java",
            Self::C => "lang-c",
            Self::Cpp => "lang-cpp",
            Self::CSharp => "lang-csharp",
            Self::Ruby => "lang-ruby",
            Self::Php => "lang-php",
            Self::Swift => "lang-swift",
            Self::Kotlin => "lang-kotlin",
            Self::Scala => "lang-scala",
            Self::Shell => "lang-shell",
            Self::Lua => "lang-lua",
            Self::Perl => "lang-perl",
            Self::R => "lang-r",
            Self::Elixir => "lang-elixir",
            Self::Haskell => "lang-haskell",
            Self::FSharp => "lang-fsharp",
            Self::Julia => "lang-julia",
            Self::PowerShell => "lang-powershell",
            Self::Proto => "lang-proto",
            Self::Css => "lang-css",
            Self::Scss => "lang-scss",
            Self::Less => "lang-less",
            Self::Clojure => "lang-clojure",
            Self::Dart => "lang-dart",
            Self::Toml => "lang-toml",
        }
    }

    /// Whether this build includes the grammar. Build with `--no-default-features` and
    /// pick `lang-*` features to trim the binary.
    #[must_use]
    pub fn is_enabled(self) -> bool {
        self.grammar().is_some()
    }

    #[allow(unreachable_patterns)] // Every arm is live only with `all-languages`.
    pub(crate) fn grammar(self) -> Option<Grammar> {
        match self {
            #[cfg(feature = "lang-python")]
            Self::Python => Some(tree_sitter_python::LANGUAGE.into()),
            #[cfg(feature = "lang-javascript")]
            Self::JavaScript => Some(tree_sitter_javascript::LANGUAGE.into()),
            #[cfg(feature = "lang-typescript")]
            Self::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            #[cfg(feature = "lang-typescript")]
            Self::Tsx => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
            #[cfg(feature = "lang-rust")]
            Self::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
            #[cfg(feature = "lang-go")]
            Self::Go => Some(tree_sitter_go::LANGUAGE.into()),
            #[cfg(feature = "lang-java")]
            Self::Java => Some(tree_sitter_java::LANGUAGE.into()),
            #[cfg(feature = "lang-c")]
            Self::C => Some(tree_sitter_c::LANGUAGE.into()),
            #[cfg(feature = "lang-cpp")]
            Self::Cpp => Some(tree_sitter_cpp::LANGUAGE.into()),
            #[cfg(feature = "lang-csharp")]
            Self::CSharp => Some(tree_sitter_c_sharp::LANGUAGE.into()),
            #[cfg(feature = "lang-ruby")]
            Self::Ruby => Some(tree_sitter_ruby::LANGUAGE.into()),
            #[cfg(feature = "lang-php")]
            Self::Php => Some(tree_sitter_php::LANGUAGE_PHP.into()),
            #[cfg(feature = "lang-swift")]
            Self::Swift => Some(tree_sitter_swift::LANGUAGE.into()),
            #[cfg(feature = "lang-kotlin")]
            Self::Kotlin => Some(tree_sitter_kotlin_ng::LANGUAGE.into()),
            #[cfg(feature = "lang-scala")]
            Self::Scala => Some(tree_sitter_scala::LANGUAGE.into()),
            #[cfg(feature = "lang-shell")]
            Self::Shell => Some(tree_sitter_bash::LANGUAGE.into()),
            #[cfg(feature = "lang-lua")]
            Self::Lua => Some(tree_sitter_lua::LANGUAGE.into()),
            #[cfg(feature = "lang-perl")]
            Self::Perl => Some(tree_sitter_perl::LANGUAGE.into()),
            #[cfg(feature = "lang-r")]
            Self::R => Some(tree_sitter_r::LANGUAGE.into()),
            #[cfg(feature = "lang-elixir")]
            Self::Elixir => Some(tree_sitter_elixir::LANGUAGE.into()),
            #[cfg(feature = "lang-haskell")]
            Self::Haskell => Some(tree_sitter_haskell::LANGUAGE.into()),
            #[cfg(feature = "lang-fsharp")]
            Self::FSharp => Some(tree_sitter_fsharp::LANGUAGE_FSHARP.into()),
            #[cfg(feature = "lang-julia")]
            Self::Julia => Some(tree_sitter_julia::LANGUAGE.into()),
            #[cfg(feature = "lang-powershell")]
            Self::PowerShell => Some(tree_sitter_powershell::LANGUAGE.into()),
            #[cfg(feature = "lang-proto")]
            Self::Proto => Some(tree_sitter_proto::LANGUAGE.into()),
            #[cfg(feature = "lang-css")]
            Self::Css => Some(tree_sitter_css::LANGUAGE.into()),
            #[cfg(feature = "lang-scss")]
            Self::Scss => Some(tree_sitter_scss::language()),
            #[cfg(feature = "lang-less")]
            Self::Less => Some(tree_sitter_less::language()),
            #[cfg(feature = "lang-clojure")]
            Self::Clojure => Some(tree_sitter_clojure_orchard::LANGUAGE.into()),
            #[cfg(feature = "lang-dart")]
            Self::Dart => Some(tree_sitter_dart::LANGUAGE.into()),
            #[cfg(feature = "lang-toml")]
            Self::Toml => Some(tree_sitter_toml_ng::LANGUAGE.into()),
            _ => None,
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
    // `#!` is the shebang that JavaScript, Rust and Kotlin grammars give a node of its own.
    line: &["//", "#!"],
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
    line_suffix: &['\'', '!'],
    line_doc: &["#'"],
    leaders: &[],
    blocks: &[],
};

const JULIA: Syntax = Syntax {
    line: &["#"],
    line_suffix: &['!'],
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
    line_suffix: &['!'],
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
