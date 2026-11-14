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
                builder.add_line(None, &literal_braces(pattern.as_ref()))?;
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
    /// A path outside the root matches on its file name alone, so an exclude is
    /// never silently skipped.
    #[must_use]
    pub fn is_excluded(&self, path: &Path) -> bool {
        let relative = self.relative(path);
        self.matchers
            .iter()
            .rev()
            .map(|m| m.matched_path_or_any_parents(&relative, false))
            .find(|m| !m.is_none())
            .is_some_and(|m| m.is_ignore())
    }

    fn relative(&self, path: &Path) -> PathBuf {
        if let Ok(rest) = path.strip_prefix(&self.root) {
            return rest.to_path_buf();
        }
        // Different spellings of the same location (relative against absolute,
        // symlinked prefixes): compare resolved paths, as 2.x did.
        if let (Ok(p), Ok(r)) = (path.canonicalize(), self.root.canonicalize())
            && let Ok(rest) = p.strip_prefix(&r)
        {
            return rest.to_path_buf();
        }
        path.file_name().map(PathBuf::from).unwrap_or_default()
    }
}

/// gitwildmatch treats `{` and `}` literally; globset would read them as
/// alternation. Wrap them in a character class (also after a backslash).
fn literal_braces(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(n @ ('{' | '}')) => {
                    out.push('[');
                    out.push(n);
                    out.push(']');
                }
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            },
            '[' if !in_class => {
                in_class = true;
                out.push(c);
            }
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            '{' | '}' if !in_class => {
                out.push('[');
                out.push(c);
                out.push(']');
            }
            _ => out.push(c),
        }
    }
    out
}
