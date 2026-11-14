//! Config-level excludes: gitignore-style patterns matched against a path relative
//! to the config root, as the 2.x `Config.exclude` did through `pathspec`.
//!
//! `pathspec`'s `match_file` tests the whole relative path against every pattern in
//! order, and the last pattern that matches decides. A pattern matches a path when
//! it matches the path or any ancestor directory. `Excludes` reproduces that: it
//! compiles each pattern alone and lets the last matching one decide, instead of
//! using git's "an excluded parent cannot be re-included" rule. The parity table in
//! `tests/fixtures/exclude_parity.tsv` is generated from `pathspec` itself.

use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::syntax::{class_end, lexical_clean};

/// Why an exclude pattern was rejected.
#[derive(Debug, thiserror::Error)]
#[error("invalid exclude pattern: {0}")]
pub struct ExcludeError(#[from] ignore::Error);

/// A compiled exclude list.
#[derive(Debug, Clone)]
pub struct Excludes {
    root: PathBuf,
    /// One matcher per pattern, in the order given. Comments and blank lines
    /// compile to matchers that match nothing.
    matchers: Vec<Gitignore>,
}

impl Excludes {
    /// Compile `patterns` (gitwildmatch syntax) anchored at `root`.
    pub fn new<S: AsRef<str>>(root: &Path, patterns: &[S]) -> Result<Self, ExcludeError> {
        let matchers = patterns
            .iter()
            .map(|pattern| {
                // The matcher gets an empty root: `is_excluded` strips the real
                // root exactly once, so a relative root cannot be stripped twice.
                let mut builder = GitignoreBuilder::new("");
                if let Some(line) = translate(pattern.as_ref()) {
                    builder.add_line(None, &line)?;
                }
                builder.build()
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            root: root.to_path_buf(),
            matchers,
        })
    }

    /// Whether `path` is excluded.
    ///
    /// A path outside the root matches nothing: `docs/contract/config.md` removes
    /// the 2.x fallback to the bare file name.
    #[must_use]
    pub fn is_excluded(&self, path: &Path) -> bool {
        let Some(relative) = self.relative(path).filter(|r| !r.as_os_str().is_empty()) else {
            return false;
        };
        self.matchers
            .iter()
            .rev()
            .map(|m| m.matched_path_or_any_parents(&relative, false))
            .find(|m| !m.is_none())
            .is_some_and(|m| m.is_ignore())
    }

    /// `path` relative to the root. Existing paths are resolved first, so
    /// symlinks and `..` cannot change which directories the matcher sees. When
    /// either side does not exist, `.` and `..` are collapsed lexically instead.
    fn relative(&self, path: &Path) -> Option<PathBuf> {
        if let (Ok(p), Ok(r)) = (path.canonicalize(), self.root.canonicalize())
            && let Ok(rest) = p.strip_prefix(&r)
        {
            return Some(rest.to_path_buf());
        }
        let lexical = |p: &Path| lexical_clean(p);
        if let Ok(rest) = lexical(path).strip_prefix(lexical(&self.root)) {
            return Some(rest.to_path_buf());
        }
        let absolute = |p: &Path| std::path::absolute(p).map(|a| lexical_clean(&a));
        let (p, r) = (absolute(path).ok()?, absolute(&self.root).ok()?);
        p.strip_prefix(r).ok().map(Path::to_path_buf)
    }
}

/// Translate gitwildmatch syntax that globset reads differently.
///
/// - `{` and `}` are literal in gitwildmatch; globset reads them as alternation.
///   Outside a class they become `[{]` and `[}]`.
/// - A character class is `[`, an optional `!` or `^`, an optional literal `]`,
///   then members up to the next `]`. It is copied as written.
/// - A `[` with no closing `]` makes `pathspec` drop the pattern, so it matches
///   nothing. `None` means that.
fn translate(pattern: &str) -> Option<String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                match chars.get(i + 1) {
                    Some(&n @ ('{' | '}')) => {
                        out.push('[');
                        out.push(n);
                        out.push(']');
                    }
                    Some(&n) => {
                        out.push('\\');
                        out.push(n);
                    }
                    None => out.push('\\'),
                }
                i += 2;
            }
            '[' => {
                let end = class_end(&chars, i)?;
                out.extend(&chars[i..end]);
                i = end;
            }
            c @ ('{' | '}') => {
                out.push('[');
                out.push(c);
                out.push(']');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(out)
}
