//! Safe summaries of pack-file problems: a location and a fixed reason, never file text.

use std::fmt;

/// What is wrong with one category file or rule, in words that hold no file text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RuleFault {
    /// The file could not be read.
    #[error("cannot read the file")]
    Read,
    /// The file is over the size limit.
    #[error("the file is over the size limit")]
    TooLarge,
    /// The pack is over a budget.
    #[error("the pack is over a budget")]
    Budget,
    /// The file does not parse as a category file.
    #[error("the file does not parse")]
    Parse,
    /// The file type is not supported.
    #[error("unsupported file type")]
    UnsupportedFormat,
    /// Two files define the same category.
    #[error("duplicate category")]
    DuplicateCategory,
    /// Two rules of a category share an id.
    #[error("duplicate rule id")]
    DuplicateRule,
    /// A pattern does not compile.
    #[error("a pattern does not compile")]
    Compile,
    /// A `bad` example does not match its rule.
    #[error("example {index}: the bad text does not match the rule")]
    BadExample {
        /// Zero-based example index.
        index: usize,
    },
    /// A `good` example matches its rule.
    #[error("example {index}: the good text matches the rule")]
    GoodExample {
        /// Zero-based example index.
        index: usize,
    },
    /// Matching hit a limit.
    #[error("matching hit a limit")]
    Match,
}

impl From<&slopvac_rules::LoadErrorKind> for RuleFault {
    fn from(kind: &slopvac_rules::LoadErrorKind) -> Self {
        use slopvac_rules::LoadErrorKind as K;
        match kind {
            K::Read(_) => Self::Read,
            K::TooLarge(_) => Self::TooLarge,
            K::BudgetExceeded { .. } => Self::Budget,
            K::UnsupportedFormat => Self::UnsupportedFormat,
            K::DuplicateCategory(_) => Self::DuplicateCategory,
            K::DuplicateRule(_) => Self::DuplicateRule,
            K::Compile(_) => Self::Compile,
            K::BadExampleDoesNotMatch { index, .. } => Self::BadExample { index: *index },
            K::GoodExampleMatches { index, .. } => Self::GoodExample { index: *index },
            K::Match(_) => Self::Match,
            _ => Self::Parse,
        }
    }
}

/// One problem in a category file: where, and a fixed reason.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RuleProblem {
    /// The pack-relative file.
    pub file: String,
    /// The one-based line, when known.
    pub line: Option<u32>,
    /// What is wrong.
    pub fault: RuleFault,
}

impl RuleProblem {
    /// A problem at `file` and `line`.
    #[must_use]
    pub fn new(file: impl Into<String>, line: Option<u32>, fault: RuleFault) -> Self {
        Self {
            file: file.into(),
            line,
            fault,
        }
    }
}

impl fmt::Display for RuleProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        write!(f, ": {}", self.fault)
    }
}

pub(crate) fn render_problems(problems: &[RuleProblem]) -> String {
    problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// What is wrong with a genre or tagger file, in words that hold no file text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ArtifactFault {
    /// The text is not well-formed YAML.
    #[error("the YAML is malformed at line {line}, column {column}")]
    Syntax {
        /// One-based line.
        line: usize,
        /// One-based column.
        column: usize,
    },
    /// The YAML does not fit the schema.
    #[error("the file does not match its schema")]
    Schema,
    /// A pattern does not compile or is over the size limit.
    #[error("a pattern is not valid")]
    Pattern,
    /// A glob is not valid.
    #[error("a glob is not valid")]
    Glob,
    /// The file breaks a rule that the schema states beyond field types.
    #[error("the file breaks a schema rule")]
    Invalid,
}

/// What a size cap counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LimitKind {
    /// Files listed in `provides`.
    #[error("files")]
    Files,
    /// Bytes of pack files.
    #[error("bytes")]
    Bytes,
    /// Rules.
    #[error("rules")]
    Rules,
}
