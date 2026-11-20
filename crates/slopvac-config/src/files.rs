//! The `files` matcher of `[[overrides]]` and `[[genres]]` blocks.

use std::path::{Component, Path};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Why a list of `files` patterns did not compile.
#[derive(Debug, thiserror::Error)]
pub enum FilesError {
    /// One pattern is not a valid gitignore-style glob.
    #[error("glob `{pattern}`: {source}")]
    Pattern {
        /// The pattern as written.
        pattern: String,
        /// The glob parser error.
        #[source]
        source: ignore::Error,
    },
    /// Every pattern parsed, but the list did not build as a set.
    #[error("{0}")]
    Set(#[source] ignore::Error),
}

/// A compiled list of `files` patterns.
///
/// Patterns use gitignore semantics: a leading `!` negates, the last matching pattern wins,
/// and a pattern that matches a directory covers the files below it.
#[derive(Debug, Clone)]
pub struct FilesMatcher(Gitignore);

impl FilesMatcher {
    /// Compile `patterns`, which are relative to `root`.
    pub fn new(patterns: &[String], root: &Path) -> Result<Self, FilesError> {
        let mut builder = GitignoreBuilder::new(root);
        for pattern in patterns {
            builder
                .add_line(None, pattern)
                .map_err(|source| FilesError::Pattern {
                    pattern: pattern.clone(),
                    source,
                })?;
        }
        builder.build().map(Self).map_err(FilesError::Set)
    }

    /// Whether the patterns select `rel`, a path relative to `root`. A path that leaves
    /// `root` at any point, or is absolute, is never selected.
    #[must_use]
    pub fn is_match(&self, rel: &Path) -> bool {
        is_under_root(rel) && self.0.matched_path_or_any_parents(rel, false).is_ignore()
    }
}

/// Whether `rel` can be matched against root-relative patterns. A path that is absolute, or
/// that climbs above the root at any point (`a/../../x`), is outside the root and matches no
/// pattern. The matcher panics on such a path, so every call into it goes through this check.
/// The walk is lexical and touches no filesystem.
pub(crate) fn is_under_root(rel: &Path) -> bool {
    let mut depth = 0usize;
    for component in rel.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => return false,
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                let Some(up) = depth.checked_sub(1) else {
                    return false;
                };
                depth = up;
            }
        }
    }
    true
}
