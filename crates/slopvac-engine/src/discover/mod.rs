//! File discovery: turn command-line targets into the ordered list of files to lint.
//!
//! Targets are files, directories or glob patterns. Directory walks use the
//! `ignore` crate and glob targets use `globset`. Output order is deterministic:
//! targets in the order given, and inside a directory the files sorted by name
//! before the sub-directories, each sorted by name.
//!
//! Rules carried over from the 2.x reference (`_expand_paths` in `pipeline.py`):
//!
//! - Directory walks skip [`SKIP_DIRS`] below the target, hidden directories are
//!   walked, and symlinks met during a walk are never followed or listed.
//! - A symlink named *as a target* is accepted (2.x #253): only symlinks found by
//!   walking are rejected.
//! - Names match case-insensitively through the [`FileKind`] registry.
//! - A target that is not selected is never dropped silently: it is recorded in
//!   [`Discovery::skipped`] (explicit files) or [`Discovery::notes`] (unsupported
//!   source files met in a comment-mode walk).
//!
//! `.gitignore` handling is off unless [`Options::use_gitignore`] is set, and it
//! applies to directory walks only: a file named explicitly is always linted. The
//! `.gitignore` files of parent directories apply to a nested target, up to the
//! filesystem root and whether or not a `.git` directory exists.

mod exclude;
mod kind;

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use globset::GlobBuilder;
use ignore::WalkBuilder;

pub use exclude::{ExcludeError, Excludes};
pub use kind::FileKind;

/// Directory names a walk never enters (below the target itself).
pub const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".venv",
    "venv",
    "node_modules",
    "vendor",
    "build",
    "dist",
    "target",
    "__pycache__",
];

/// Which kinds of file a run selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selection {
    /// Prose documents only (Markdown, MDX, HTML, RST, plain text).
    #[default]
    Prose,
    /// Comments in source and TOML files only.
    CodeComments,
    /// Both of the above, for callers that decide per file.
    Both,
}

/// Discovery settings supplied by the caller.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Which file kinds to select.
    pub selection: Selection,
    /// Honour `.gitignore` files while walking directories.
    pub use_gitignore: bool,
}

/// A file chosen for linting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    /// Path as built from the target, not canonicalised.
    pub path: PathBuf,
    /// What the file holds.
    pub kind: FileKind,
}

/// A target that was asked for and not linted, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The skipped path.
    pub path: PathBuf,
    /// Why it was skipped.
    pub reason: String,
}

/// The result of [`discover`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    /// Files to lint, in deterministic order, without duplicates.
    pub files: Vec<DiscoveredFile>,
    /// Explicit files that no selected kind covers.
    pub skipped: Vec<Skipped>,
    /// Informational notes; they do not change the exit status.
    pub notes: Vec<String>,
}

/// Why discovery failed.
#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    /// A target is not a file, a directory or a pattern with a match.
    #[error("no such file or directory: {0}")]
    NoSuchTarget(String),
    /// An explicit file is not a supported source language in comment mode.
    #[error("{path}: {message}")]
    UnsupportedSource {
        /// The offending file.
        path: PathBuf,
        /// What is unsupported and what is supported.
        message: String,
    },
    /// A glob target does not parse.
    #[error("invalid glob {pattern}: {source}")]
    InvalidGlob {
        /// The pattern as given.
        pattern: String,
        /// The parse error.
        source: globset::Error,
    },
    /// The directory walk hit an I/O error.
    #[error("walking {path}: {source}")]
    Walk {
        /// The walk root.
        path: PathBuf,
        /// The underlying error.
        source: ignore::Error,
    },
}

/// Expand `targets` into the files to lint.
pub fn discover<S: AsRef<str>>(
    targets: &[S],
    options: Options,
) -> Result<Discovery, DiscoverError> {
    let mut out = Discovery::default();

    for target in targets {
        let target = target.as_ref();
        let path = Path::new(target);
        if path.is_dir() {
            walk_dir(path, options, &mut out)?;
        } else if path.is_file() {
            explicit(path, options.selection, &mut out)?;
        } else {
            for matched in expand_glob(target)? {
                explicit(&matched, options.selection, &mut out)?;
            }
        }
    }

    let mut seen = HashSet::new();
    out.files
        .retain(|f| seen.insert(std::fs::canonicalize(&f.path).unwrap_or_else(|_| f.path.clone())));
    Ok(out)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The kind a file has under `selection`, or `None` when it is not selected.
fn select(name: &str, selection: Selection) -> Option<FileKind> {
    match selection {
        Selection::Prose => FileKind::prose(name),
        Selection::CodeComments => FileKind::comment(name),
        Selection::Both => FileKind::from_file_name(name),
    }
}

fn explicit(path: &Path, selection: Selection, out: &mut Discovery) -> Result<(), DiscoverError> {
    let name = file_name(path);
    if selection == Selection::CodeComments && FileKind::comment(&name).is_none() {
        return Err(DiscoverError::UnsupportedSource {
            path: path.to_path_buf(),
            message: unsupported_message(&name),
        });
    }
    match select(&name, selection) {
        Some(kind) => out.files.push(DiscoveredFile {
            path: path.to_path_buf(),
            kind,
        }),
        None => out.skipped.push(Skipped {
            path: path.to_path_buf(),
            reason: "not a file kind this run lints".to_owned(),
        }),
    }
    Ok(())
}

fn unsupported_message(name: &str) -> String {
    let lower = name.to_lowercase();
    let shown = match lower.rfind('.').filter(|&i| i > 0) {
        Some(i) => lower[i..].to_owned(),
        None if lower.is_empty() => "<extensionless>".to_owned(),
        None => lower,
    };
    format!(
        "unsupported source language for '{shown}'; supported extensions: {}",
        kind::supported_extensions()
    )
}

fn walk_dir(root: &Path, options: Options, out: &mut Discovery) -> Result<(), DiscoverError> {
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .parents(options.use_gitignore)
        .git_ignore(options.use_gitignore)
        .filter_entry(|entry| {
            let Some(file_type) = entry.file_type() else {
                return false;
            };
            if entry.depth() == 0 {
                return true;
            }
            if file_type.is_symlink() {
                return false;
            }
            !(file_type.is_dir()
                && SKIP_DIRS.contains(&entry.file_name().to_string_lossy().as_ref()))
        });
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in builder.build() {
        let entry = entry.map_err(|source| DiscoverError::Walk {
            path: root.to_path_buf(),
            source,
        })?;
        if entry.file_type().is_some_and(|t| t.is_file()) {
            files.push(entry.into_path());
        }
    }
    // Files of a directory come before its sub-directories, as in the 2.x walk.
    files.sort_by_key(|path| walk_order(path));

    for path in files {
        let name = file_name(&path);
        let lower = name.to_lowercase();
        if options.selection != Selection::CodeComments
            && (lower == "slopvac.toml" || lower == ".slopvac.toml")
        {
            continue;
        }
        if let Some(kind) = select(&name, options.selection) {
            out.files.push(DiscoveredFile { path, kind });
        } else if options.selection != Selection::Prose && FileKind::prose(&name).is_none() {
            out.notes.push(format!(
                "unsupported source language skipped: {} ({})",
                path.display(),
                unsupported_message(&name)
            ));
        }
    }
    Ok(())
}

/// Sort key giving 2.x `os.walk` order: at each level, files by name, then
/// directories by name, depth first.
fn walk_order(path: &Path) -> Vec<(bool, std::ffi::OsString)> {
    let parts: Vec<_> = path.components().collect();
    let last = parts.len().saturating_sub(1);
    parts
        .iter()
        .enumerate()
        .map(|(i, c)| (i != last, c.as_os_str().to_owned()))
        .collect()
}

/// Match a non-existent target as a glob, relative to the working directory or
/// absolute. Only files match; results are sorted.
fn expand_glob(pattern: &str) -> Result<Vec<PathBuf>, DiscoverError> {
    let none = || DiscoverError::NoSuchTarget(pattern.to_owned());
    // `./docs/*.md` and `docs/*.md` are the same pattern; candidates below are
    // normalised the same way.
    let normalised: PathBuf = Path::new(pattern)
        .components()
        .filter(|c| *c != Component::CurDir)
        .collect();
    let pattern = normalised.to_string_lossy().into_owned();
    let pattern = pattern.as_str();
    let glob = GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|source| DiscoverError::InvalidGlob {
            pattern: pattern.to_owned(),
            source,
        })?
        .compile_matcher();

    // Walk from the longest leading run of components without glob syntax.
    let mut base = PathBuf::new();
    for component in Path::new(pattern).components() {
        let text = component.as_os_str().to_string_lossy();
        if text.contains(['*', '?', '[', '{']) {
            break;
        }
        base.push(component);
    }
    if base.as_os_str().is_empty() {
        base.push(".");
    }
    if !base.is_dir() {
        return Err(none());
    }

    let mut builder = WalkBuilder::new(&base);
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false);
    let mut matches: Vec<PathBuf> = Vec::new();
    for entry in builder.build() {
        let entry = entry.map_err(|source| DiscoverError::Walk {
            path: base.clone(),
            source,
        })?;
        let path = entry.path();
        let candidate: PathBuf = path
            .components()
            .filter(|c| *c != Component::CurDir)
            .collect();
        if glob.is_match(&candidate) && path.is_file() {
            matches.push(candidate);
        }
    }
    if matches.is_empty() {
        return Err(none());
    }
    matches.sort();
    Ok(matches)
}

#[cfg(test)]
mod tests;
