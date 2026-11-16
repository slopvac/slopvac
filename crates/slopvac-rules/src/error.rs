//! Errors that name the file and line they come from.

use std::fmt;
use std::path::PathBuf;

/// What went wrong with one file or rule.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadErrorKind {
    /// The file or directory could not be read.
    Read(String),
    /// The file does not parse as a rule or category file.
    Parse(String),
    /// The extension names no supported format (`json`, `yaml`, `yml`, `toml`).
    UnsupportedFormat,
    /// Two files define the same category.
    DuplicateCategory(String),
    /// Two rules of one category share an id.
    DuplicateRule(String),
    /// A lexical rule's pattern does not compile.
    Compile(String),
    /// An example's `bad` text does not match its rule.
    BadExampleDoesNotMatch {
        /// Zero-based example index.
        index: usize,
        /// The example text.
        text: String,
    },
    /// An example's `good` text matches its rule.
    GoodExampleMatches {
        /// Zero-based example index.
        index: usize,
        /// The example text.
        text: String,
    },
    /// Matching an example hit a limit.
    Match(String),
}

impl fmt::Display for LoadErrorKind {
    #[expect(
        clippy::use_debug,
        reason = "Debug quotes and escapes the example text, which can hold control characters"
    )]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(e) => write!(f, "cannot read: {e}"),
            Self::Parse(e) => f.write_str(e),
            Self::UnsupportedFormat => {
                f.write_str("unsupported file type (expected json, yaml, yml or toml)")
            }
            Self::DuplicateCategory(id) => write!(f, "duplicate category `{id}`"),
            Self::DuplicateRule(id) => write!(f, "duplicate rule id `{id}` in its category"),
            Self::Compile(e) => write!(f, "pattern does not compile: {e}"),
            Self::BadExampleDoesNotMatch { index, text } => write!(
                f,
                "example {index} 'bad' text does not match its own pattern: {text:?}"
            ),
            Self::GoodExampleMatches { index, text } => write!(
                f,
                "example {index} 'good' text matches the pattern, so the rule fires on its own fix: {text:?}"
            ),
            Self::Match(e) => write!(f, "matching failed: {e}"),
        }
    }
}

/// One problem, with where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    /// The file.
    pub file: PathBuf,
    /// The one-based line, when it is known. A minified one-line file reports line 1 for
    /// parse errors and none for a rule.
    pub line: Option<u32>,
    /// The rule (`<category>.<rule>`), when the problem belongs to one.
    pub rule: Option<String>,
    /// What went wrong.
    pub kind: LoadErrorKind,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file.display())?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        f.write_str(": ")?;
        if let Some(rule) = &self.rule {
            write!(f, "{rule}: ")?;
        }
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for LoadError {}

/// Every problem found while loading, so one broken rule reports next to the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadErrors(pub Vec<LoadError>);

impl LoadErrors {
    /// The problems, in the order found.
    #[must_use]
    pub fn errors(&self) -> &[LoadError] {
        &self.0
    }
}

impl fmt::Display for LoadErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LoadErrors {}

impl From<LoadError> for LoadErrors {
    fn from(e: LoadError) -> Self {
        Self(vec![e])
    }
}
