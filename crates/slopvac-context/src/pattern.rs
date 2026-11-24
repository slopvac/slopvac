//! Thin PCRE2 wrapper so tagger code does not touch the crate API directly.

use slopvac_pcre2::{MatchError, Options, Regex};

use crate::Error;

/// Longest authored pattern a data pack may carry, in bytes as written
/// (`docs/contract/threat-model.md`). The `slopvac-pcre2` limit is larger because it also serves
/// patterns that code builds.
pub const MAX_PATTERN_BYTES: usize = 8192;

/// Refuse an authored pattern longer than [`MAX_PATTERN_BYTES`], counted as written. A pattern
/// that code generates, such as the alternation of a `words` matcher, is not checked here and
/// keeps the `slopvac-pcre2` compile limit.
pub(crate) fn check_authored(source: &str) -> Result<(), Error> {
    if source.len() > MAX_PATTERN_BYTES {
        return Err(Error::PatternTooLong {
            len: source.len(),
            max: MAX_PATTERN_BYTES,
        });
    }
    Ok(())
}

/// A pattern budget refused to go on, so the compile stopped. The budget keeps the reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetStop;

/// Asked around each pattern a tagger compiles, so a compile stops at the first pattern that
/// crosses a budget and no later pattern compiles. The caller keeps the reason for a stop.
pub trait PatternBudget {
    /// Called before a pattern of `source_bytes` compiles.
    ///
    /// # Errors
    /// [`BudgetStop`] when the pattern would cross the budget.
    fn reserve(&mut self, source_bytes: usize) -> Result<(), BudgetStop>;

    /// Called after a pattern of `source_bytes` compiled to `compiled_bytes`, as PCRE2 reports
    /// its size. It settles the reservation.
    ///
    /// # Errors
    /// [`BudgetStop`] when the compiled size crossed the budget.
    fn settle(&mut self, source_bytes: usize, compiled_bytes: usize) -> Result<(), BudgetStop>;
}

/// A budget with no limit.
pub(crate) struct Unlimited;

impl PatternBudget for Unlimited {
    fn reserve(&mut self, _source_bytes: usize) -> Result<(), BudgetStop> {
        Ok(())
    }

    fn settle(&mut self, _source_bytes: usize, _compiled_bytes: usize) -> Result<(), BudgetStop> {
        Ok(())
    }
}

/// A compiled PCRE2 pattern (UTF mode, interpreter under the `slopvac-pcre2` limits).
#[derive(Debug, Clone)]
pub struct Pattern {
    source: String,
    re: Regex,
}

impl Pattern {
    /// Compile a case-sensitive pattern.
    pub fn new(source: &str) -> Result<Self, Error> {
        Self::build(source, false)
    }

    /// Compile a case-insensitive pattern.
    pub fn new_caseless(source: &str) -> Result<Self, Error> {
        Self::build(source, true)
    }

    /// Compile under `budget`: reserve the source bytes first and settle the compiled size
    /// after, so the first pattern that crosses the budget stops the compile.
    pub(crate) fn new_charged(
        source: &str,
        caseless: bool,
        budget: &mut dyn PatternBudget,
    ) -> Result<Self, Error> {
        let stopped =
            |_: BudgetStop| Error::Parse("a pattern budget stopped the compile".to_owned());
        budget.reserve(source.len()).map_err(stopped)?;
        let pattern = Self::build(source, caseless)?;
        budget
            .settle(source.len(), pattern.compiled_size())
            .map_err(stopped)?;
        Ok(pattern)
    }

    fn build(source: &str, caseless: bool) -> Result<Self, Error> {
        let re =
            Regex::new(source, Options::new().caseless(caseless)).map_err(|e| Error::Pattern {
                pattern: source.to_string(),
                source: e,
            })?;
        Ok(Self {
            source: source.to_string(),
            re,
        })
    }

    /// Size in bytes of the compiled pattern, as PCRE2 reports it.
    #[must_use]
    pub fn compiled_size(&self) -> usize {
        self.re.compiled_size()
    }

    /// The pattern source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// True when the pattern matches anywhere in `text`. A PCRE2 execution error, such as a
    /// match or JIT stack limit, is an error and never a miss.
    pub fn is_match(&self, text: &str) -> Result<bool, Error> {
        self.re.is_match(text).map_err(|e| self.exec_error(&e))
    }

    /// Byte range of the first match.
    pub fn find(&self, text: &str) -> Result<Option<std::ops::Range<usize>>, Error> {
        self.re.find(text).map_err(|e| self.exec_error(&e))
    }

    fn exec_error(&self, e: &MatchError) -> Error {
        Error::Match {
            pattern: self.source.clone(),
            source: e.clone(),
        }
    }
}

/// A path as patterns see it: forward slashes, no drive prefix, no leading slash, and `.` and
/// `name/..` components collapsed lexically (a leading `..` stays). Windows
/// paths then match the same patterns as Unix paths, and an absolute path never reaches a
/// matcher that expects a path relative to a root.
#[must_use]
pub fn normalize_path(path: &str) -> String {
    let forward = path.replace('\\', "/");
    let bytes = forward.as_bytes();
    let no_drive = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        &forward[2..]
    } else {
        forward.as_str()
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in no_drive.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}
