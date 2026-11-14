//! The file-kind registry: the single table that maps a file name to the kind of
//! document it holds. Directory walks and explicit targets both use it, so a kind
//! cannot be listed in one place and missing from the other.

/// Prose extensions, matched against the end of the lower-cased file name.
const PROSE: &[(&str, FileKind)] = &[
    (".md", FileKind::Markdown),
    (".markdown", FileKind::Markdown),
    (".mdx", FileKind::Mdx),
    (".txt", FileKind::Text),
    (".rst", FileKind::Rst),
    (".html", FileKind::Html),
];

/// Source extensions and the comment language each one selects.
const SOURCE_EXTENSIONS: &[(&str, &str)] = &[
    (".py", "python"),
    (".pyi", "python"),
    (".pyw", "python"),
    (".js", "javascript"),
    (".jsx", "javascript"),
    (".mjs", "javascript"),
    (".cjs", "javascript"),
    (".ts", "typescript"),
    (".tsx", "typescript"),
    (".rs", "rust"),
    (".go", "go"),
    (".java", "java"),
    (".c", "c"),
    (".h", "c"),
    (".cc", "cpp"),
    (".cpp", "cpp"),
    (".cxx", "cpp"),
    (".hh", "cpp"),
    (".hpp", "cpp"),
    (".hxx", "cpp"),
    (".cs", "csharp"),
    (".rb", "ruby"),
    (".php", "php"),
    (".swift", "swift"),
    (".kt", "kotlin"),
    (".kts", "kotlin"),
    (".scala", "scala"),
    (".sh", "shell"),
    (".bash", "shell"),
    (".zsh", "shell"),
    (".lua", "lua"),
    (".pl", "perl"),
    (".pm", "perl"),
    (".r", "r"),
    (".ex", "elixir"),
    (".exs", "elixir"),
    (".hs", "haskell"),
    (".fs", "fsharp"),
    (".fsx", "fsharp"),
    (".jl", "julia"),
    (".ps1", "powershell"),
    (".psm1", "powershell"),
    (".proto", "proto"),
    (".css", "css"),
    (".scss", "scss"),
    (".less", "less"),
    (".clj", "clojure"),
    (".cljs", "clojure"),
    (".dart", "dart"),
];

/// Extensionless source files, keyed by lower-cased name.
const SOURCE_FILENAMES: &[(&str, &str)] = &[
    ("sconstruct", "python"),
    ("gemfile", "ruby"),
    ("rakefile", "ruby"),
    ("brewfile", "ruby"),
];

/// What a discovered file contains.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    /// `.md` or `.markdown`.
    Markdown,
    /// `.mdx`.
    Mdx,
    /// `.html`.
    Html,
    /// `.rst`. Always discovered; whether a converter exists is the linter's concern.
    Rst,
    /// `.txt`.
    Text,
    /// A TOML file, linted for comments only.
    Toml,
    /// A source file, linted for comments only. Holds the comment language.
    Source(&'static str),
}

impl FileKind {
    /// Whether the file is a prose document rather than a comment-only file.
    #[must_use]
    pub fn is_prose(&self) -> bool {
        !matches!(self, Self::Toml | Self::Source(_))
    }

    /// Classify a file name (not a path), ignoring case. `None` means unsupported.
    #[must_use]
    pub fn from_file_name(name: &str) -> Option<Self> {
        Self::prose(name).or_else(|| Self::comment(name))
    }

    pub(super) fn prose(name: &str) -> Option<Self> {
        let lower = name.to_lowercase();
        PROSE
            .iter()
            .find(|(ext, _)| lower.ends_with(ext))
            .map(|(_, kind)| kind.clone())
    }

    // `lower` is already lower-cased; Path::extension would miss a bare `.toml`.
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    pub(super) fn comment(name: &str) -> Option<Self> {
        let lower = name.to_lowercase();
        if let Some((_, lang)) = SOURCE_FILENAMES.iter().find(|(n, _)| *n == lower) {
            return Some(Self::Source(lang));
        }
        if lower.ends_with(".toml") {
            return Some(Self::Toml);
        }
        source_language(&lower).map(Self::Source)
    }
}

/// The comment language for a lower-cased file name, by its final extension. A
/// leading dot alone (`.py`) is a hidden file, not an extension.
fn source_language(lower: &str) -> Option<&'static str> {
    let dot = lower.rfind('.').filter(|&i| i > 0)?;
    let ext = &lower[dot..];
    SOURCE_EXTENSIONS
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, lang)| *lang)
}

/// Every source extension, sorted, for error messages.
pub(super) fn supported_extensions() -> String {
    let mut all: Vec<&str> = SOURCE_EXTENSIONS.iter().map(|(e, _)| *e).collect();
    all.sort_unstable();
    all.join(", ")
}
