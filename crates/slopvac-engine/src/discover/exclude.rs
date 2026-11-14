//! Config-level excludes: gitignore-style patterns matched against a path relative
//! to the config root, as the 2.x `Config.exclude` did through `pathspec`.

use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Why an exclude pattern was rejected.
#[derive(Debug, thiserror::Error)]
#[error("invalid exclude pattern: {0}")]
pub struct ExcludeError(#[from] ignore::Error);

/// A compiled exclude list.
#[derive(Debug, Clone)]
pub struct Excludes {
    root: std::path::PathBuf,
    matcher: Gitignore,
}

impl Excludes {
    /// Compile `patterns` (gitignore syntax) anchored at `root`.
    pub fn new<S: AsRef<str>>(root: &Path, patterns: &[S]) -> Result<Self, ExcludeError> {
        let mut builder = GitignoreBuilder::new(root);
        for pattern in patterns {
            builder.add_line(None, pattern.as_ref())?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            matcher: builder.build()?,
        })
    }

    /// Whether `path` (or any parent directory below the root) is excluded.
    ///
    /// A path outside the root matches on its file name alone, so an exclude is
    /// never silently skipped.
    #[must_use]
    pub fn is_excluded(&self, path: &Path) -> bool {
        let relative = path.strip_prefix(&self.root).ok().map_or_else(
            || {
                path.file_name()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_default()
            },
            Path::to_path_buf,
        );
        self.matcher
            .matched_path_or_any_parents(&relative, false)
            .is_ignore()
    }
}
