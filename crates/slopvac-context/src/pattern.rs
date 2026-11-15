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

    /// True when the pattern matches anywhere in `text`. A PCRE2 execution error, such as a
    /// match or JIT stack limit, is an error and never a miss.
    pub fn is_match(&self, text: &str) -> Result<bool, Error> {
        self.re
            .is_match(text.as_bytes())
            .map_err(|e| self.exec_error(&e))
    }

    /// Byte range of the first match.
    pub fn find(&self, text: &str) -> Result<Option<std::ops::Range<usize>>, Error> {
        self.re
            .find(text.as_bytes())
            .map(|m| m.map(|m| m.start()..m.end()))
            .map_err(|e| self.exec_error(&e))
    }

    fn exec_error(&self, e: &pcre2::Error) -> Error {
        Error::Match {
            pattern: self.source.clone(),
            message: e.to_string(),
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
