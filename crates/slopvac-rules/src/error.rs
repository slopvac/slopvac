//! Errors that name the file and line they come from.

use std::fmt;
use std::path::PathBuf;

use crate::messages;

/// Which pack budget a load passed. The list can grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackBudget {
    /// Rule and category files, in files: [`MAX_PACK_FILES`](crate::MAX_PACK_FILES).
    Files,
    /// Directory entries of any kind, in entries: [`MAX_PACK_ENTRIES`](crate::MAX_PACK_ENTRIES).
    Entries,
    /// Bytes read, in bytes: [`MAX_PACK_BYTES`](crate::MAX_PACK_BYTES).
    Bytes,
    /// Tokens or substitutions in one rule, in entries:
    /// [`MAX_RULE_ENTRIES`](crate::MAX_RULE_ENTRIES).
    RuleEntries,
    /// Bytes of pack files that all packs of one run supply together. The host sets the limit.
    RunBytes,
    /// Rules that all packs of one run declare together. The host sets the limit.
    RunRules,
    /// Bytes of compiled patterns that all packs of one run hold together: the size PCRE2
    /// reports for each pattern. The host sets the limit.
    RunPatternBytes,
}

impl PackBudget {
    /// The `data.budget` text, such as `plugin-files`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Files => "plugin-files",
            Self::Entries => "plugin-entries",
            Self::Bytes => "plugin-bytes",
            Self::RuleEntries => "rule-entries",
            Self::RunBytes => "plugin-run-bytes",
            Self::RunRules => "plugin-run-rules",
            Self::RunPatternBytes => "plugin-run-pattern-bytes",
        }
    }

    /// The stable error code of a refusal over this budget, read from the message catalog.
    #[must_use]
    pub fn code(self) -> &'static str {
        use slopvac_messages::Catalog as _;
        crate::messages::budget_exceeded(self, 0).code()
    }

    /// The text of a refusal over this budget, with `limit` in the unit of the budget.
    #[must_use]
    pub fn message(self, limit: u64) -> String {
        crate::messages::budget_exceeded(self, limit).to_string()
    }

    /// The unit of the limit.
    #[must_use]
    pub fn unit(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Entries | Self::RuleEntries => "entries",
            Self::Bytes | Self::RunBytes | Self::RunPatternBytes => "bytes",
            Self::RunRules => "rules",
        }
    }
}

/// What went wrong with one file or rule.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LoadErrorKind {
    /// The file or directory could not be read.
    #[error("{}", messages::read_failed(.0))]
    Read(String),
    /// The file is larger than [`MAX_RULE_FILE_BYTES`](crate::MAX_RULE_FILE_BYTES), the given
    /// number of bytes. Its stable code is `rules-file-too-large`.
    #[error("{}", messages::file_too_large(*.0))]
    TooLarge(u64),
    /// The pack directory holds more files, or more bytes in total, than
    /// [`MAX_PACK_FILES`](crate::MAX_PACK_FILES) or [`MAX_PACK_BYTES`](crate::MAX_PACK_BYTES)
    /// allow, or one rule lists more than [`MAX_RULE_ENTRIES`](crate::MAX_RULE_ENTRIES) tokens or
    /// substitutions. Its stable code is `rules-budget-exceeded`.
    #[error("{}", messages::budget_exceeded(*.budget, *.limit))]
    BudgetExceeded {
        /// The budget that was passed.
        budget: PackBudget,
        /// The limit, in the unit of `budget`.
        limit: u64,
        /// The pack directory, or for `rule-entries` the rule file, as a display path, never
        /// absolute: relative to the working directory, or `<external>/<name>` outside it.
        path: String,
    },
    /// An authored pattern is longer than
    /// [`MAX_AUTHORED_PATTERN_BYTES`](crate::MAX_AUTHORED_PATTERN_BYTES). The loader refuses the
    /// whole load, as for [`LoadErrorKind::TooLarge`]. Its stable code is
    /// `rules-pattern-too-long`. The error never holds the pattern text. Its line is the line where
    /// the rule starts, because the loader keeps no position for a value inside a rule.
    #[error("{}", messages::pattern_too_long(*.len, *.limit))]
    PatternTooLong {
        /// The pattern length as written, in UTF-8 bytes.
        len: usize,
        /// The limit in bytes, `data.limit`.
        limit: usize,
    },
    /// The file does not parse as a rule or category file.
    #[error("{}", messages::parse_failed(.0))]
    Parse(String),
    /// The extension names no supported format (`json`, `yaml`, `yml`, `toml`).
    #[error("{}", messages::RulesMessage::UnsupportedFormat)]
    UnsupportedFormat,
    /// Two files define the same category.
    #[error("{}", messages::duplicate_category(.0))]
    DuplicateCategory(String),
    /// Two rules of one category share an id.
    #[error("{}", messages::duplicate_rule(.0))]
    DuplicateRule(String),
    /// A lexical rule's pattern does not compile.
    #[error("{}", messages::pattern_invalid(.0))]
    Compile(String),
    /// An example's `bad` text does not match its rule.
    #[error("{}", messages::bad_example(*.index, .text))]
    BadExampleDoesNotMatch {
        /// Zero-based example index.
        index: usize,
        /// The example text.
        text: String,
    },
    /// An example's `good` text matches its rule.
    #[error("{}", messages::good_example(*.index, .text))]
    GoodExampleMatches {
        /// Zero-based example index.
        index: usize,
        /// The example text.
        text: String,
    },
    /// Matching an example hit a limit.
    #[error("{}", messages::match_failed(.0))]
    Match(String),
}

impl LoadErrorKind {
    /// The stable code of a refusal that has one of its own: a file over the size cap.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        use slopvac_messages::Catalog as _;
        match self {
            Self::TooLarge(cap) => Some(messages::file_too_large(*cap).code()),
            _ => None,
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
