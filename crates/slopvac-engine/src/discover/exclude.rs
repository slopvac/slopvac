//! Config-level excludes: gitignore-style patterns matched against a path relative
//! to the config root, as the 2.x `Config.exclude` did through `pathspec`.
//!
//! `pathspec`'s `match_file` tests the whole relative path against every pattern in
//! order, and the last pattern that matches decides. A pattern matches a path when
//! it matches the path or any ancestor directory. `Excludes` reproduces that: it
//! compiles each pattern alone and lets the last matching one decide, instead of
//! using git's "an excluded parent cannot be re-included" rule. The parity table in
//! `tests/fixtures/exclude_parity.tsv` is generated from `pathspec` itself.
//!
//! Paths are matched as the project sees them. The lexical path relative to the
//! root decides, so a symlink is excluded by its own repository path and not by
//! the location it points to. A path that is lexically outside the root is tried
//! again after resolving symlinks, so an aliased root works. A path outside the
//! root both ways matches nothing, and the root itself is never excluded.

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

    /// `path` relative to the root, or `None` when it is outside.
    ///
    /// The lexical path decides first (`.` and `..` collapsed, no file system
    /// access), because users write excludes in terms of the paths they see in the
    /// repository: `/linked.md` excludes `repo/linked.md` even when that symlink
    /// points elsewhere. Only a path that is lexically outside the root gets a
    /// second try through resolved paths, which makes an aliased root (a symlink
    /// to the project directory) work.
    fn relative(&self, path: &Path) -> Option<PathBuf> {
        if let Ok(rest) = lexical_clean(path).strip_prefix(lexical_clean(&self.root)) {
            return Some(rest.to_path_buf());
        }
        let absolute = |p: &Path| std::path::absolute(p).map(|a| lexical_clean(&a));
        if let (Ok(p), Ok(r)) = (absolute(path), absolute(&self.root))
            && let Ok(rest) = p.strip_prefix(r)
        {
            return Some(rest.to_path_buf());
        }
        let (p, r) = (path.canonicalize().ok()?, self.root.canonicalize().ok()?);
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
    while let Some(&current) = chars.get(i) {
        match current {
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
                out.extend(chars.get(i..end).unwrap_or_default());
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
