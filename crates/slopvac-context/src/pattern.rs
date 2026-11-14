//! Thin PCRE2 wrapper so tagger code does not touch the crate API directly.

use pcre2::bytes::{Regex, RegexBuilder};

use crate::Error;

/// A compiled PCRE2 pattern (UTF mode, JIT when available).
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

    fn build(source: &str, caseless: bool) -> Result<Self, Error> {
        let re = RegexBuilder::new()
            .utf(true)
            .caseless(caseless)
            .jit_if_available(true)
            .build(source)
            .map_err(|e| Error::Pattern {
                pattern: source.to_string(),
                message: e.to_string(),
            })?;
        Ok(Self {
            source: source.to_string(),
            re,
        })
    }

    /// The pattern source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// True when the pattern matches anywhere in `text`.
    #[must_use]
    pub fn is_match(&self, text: &str) -> bool {
        self.re.is_match(text.as_bytes()).unwrap_or(false)
    }

    /// Byte range of the first match.
    #[must_use]
    pub fn find(&self, text: &str) -> Option<std::ops::Range<usize>> {
        self.re
            .find(text.as_bytes())
            .ok()
            .flatten()
            .map(|m| m.start()..m.end())
    }
}
