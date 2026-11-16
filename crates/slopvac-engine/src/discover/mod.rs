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
//! - A `.drawio.png` is out of scope for 3.0. It is never listed in
//!   [`Discovery::files`]; [`Discovery::notices`] holds an `info` notice with the code
//!   [`DIAGRAM_FORMAT_UNSUPPORTED`] and the path, so the skip is visible.
//!
//! `.gitignore` handling is off unless [`Options::use_gitignore`] is set, and it
//! applies to directory walks only: a file named explicitly is always linted.
//! Inside a git repository (the nearest ancestor with a `.git` directory or file)
//! the `.gitignore` files from the repository root down and `.git/info/exclude`
//! apply to a nested target; nothing above the root does, and `core.excludesFile`
//! is not read. Outside a repository only `.gitignore` files in the target and
//! below apply.
//!
//! [`Options::excludes`] applies config `exclude` patterns to every collected
//! file, explicit ones included; an excluded explicit file lands in
//! [`Discovery::skipped`].

mod exclude;
mod kind;
mod syntax;

use std::collections::{BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};

use globset::GlobBuilder;
use ignore::WalkBuilder;
use slopvac_diagrams::Format;
use slopvac_model::{KebabName, NonEmptyString, Notice, NoticeLevel};

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
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Which file kinds to select.
    pub selection: Selection,
    /// Honour `.gitignore` files while walking directories.
    pub use_gitignore: bool,
    /// Config `exclude` patterns. They apply to every collected file, named
    /// explicitly or found by a walk or glob, as `Config.is_excluded` did in 2.x.
    /// An excluded explicit file is reported in [`Discovery::skipped`].
    pub excludes: Option<Excludes>,
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
    /// Files left out because their format is not supported, as report notices. Such a
    /// file is never in [`Discovery::files`], so it cannot count as clean or scored.
    pub notices: Vec<Notice>,
}

/// The notice code for a diagram file whose format 3.0 does not lint.
pub const DIAGRAM_FORMAT_UNSUPPORTED: &str = "diagram-format-unsupported";

/// The `info` notice for `path` when its diagram format is not linted in 3.0
/// (`.drawio.png`), else `None`. `path` is shown as given.
#[must_use]
pub fn unsupported_diagram_notice(path: &str) -> Option<Notice> {
    if Format::from_path(path) != Some(Format::DrawioPng) {
        return None;
    }
    Some(Notice {
        code: KebabName::new(DIAGRAM_FORMAT_UNSUPPORTED).expect("the code is kebab-case"),
        level: NoticeLevel::Info,
        message: NonEmptyString::new(".drawio.png isn't linted in 3.0; export as .drawio.svg")
            .expect("the message is not empty"),
        path: Some(path.to_owned()),
    })
}

fn push_unsupported(path: &Path, out: &mut Discovery) -> bool {
    match unsupported_diagram_notice(&path.to_string_lossy()) {
        Some(notice) => {
            out.notices.push(notice);
            true
        }
        None => false,
    }
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
    /// A glob target holds more brace alternatives than discovery will expand.
    #[error("glob {pattern} expands to more than {limit} alternatives")]
    TooManyAlternatives {
        /// The pattern as given.
        pattern: String,
        /// The expansion cap.
        limit: usize,
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
#[allow(clippy::needless_pass_by_value)] // callers build `Options` inline
pub fn discover<S: AsRef<str>>(
    targets: &[S],
    options: Options,
) -> Result<Discovery, DiscoverError> {
    let mut out = Discovery::default();

    for target in targets {
        let target = target.as_ref();
        let cleaned = syntax::resolve_target(Path::new(target));
        let path = cleaned.as_path();
        if path.is_dir() {
            walk_dir(path, &options, &mut out)?;
        } else if path.is_file() {
            explicit(path, &options, &mut out)?;
        } else {
            for matched in expand_glob(target)? {
                explicit(&matched, &options, &mut out)?;
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

fn is_excluded(options: &Options, path: &Path) -> bool {
    options
        .excludes
        .as_ref()
        .is_some_and(|e| e.is_excluded(path))
}

fn explicit(path: &Path, options: &Options, out: &mut Discovery) -> Result<(), DiscoverError> {
    let selection = options.selection;
    let name = file_name(path);
    if selection == Selection::CodeComments && FileKind::comment(&name).is_none() {
        return Err(DiscoverError::UnsupportedSource {
            path: path.to_path_buf(),
            message: unsupported_message(&name),
        });
    }
    if is_excluded(options, path) {
        out.skipped.push(Skipped {
            path: path.to_path_buf(),
            reason: "matches an exclude pattern".to_owned(),
        });
        return Ok(());
    }
    if push_unsupported(path, out) {
        return Ok(());
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

fn walk_dir(root: &Path, options: &Options, out: &mut Discovery) -> Result<(), DiscoverError> {
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false);
    configure_gitignore(&mut builder, root, options.use_gitignore);
    builder.filter_entry(|entry| {
        let Some(file_type) = entry.file_type() else {
            return false;
        };
        if entry.depth() == 0 {
            return true;
        }
        if file_type.is_symlink() {
            return false;
        }
        !(file_type.is_dir() && SKIP_DIRS.contains(&entry.file_name().to_string_lossy().as_ref()))
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
        if is_excluded(options, &path) {
            continue;
        }
        if push_unsupported(&path, out) {
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

/// Nearest ancestor of `target` (or `target` itself) holding a `.git` entry. A
/// `.git` file counts, as in worktrees and submodules.
fn repository_root(target: &Path) -> Option<PathBuf> {
    let absolute = std::fs::canonicalize(target)
        .or_else(|_| std::path::absolute(target))
        .ok()?;
    absolute
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Inside a repository: `.gitignore` files from the repository root down, plus
/// `.git/info/exclude`; nothing above the root. `core.excludesFile` is left out
/// because it depends on the user's machine. Outside a repository: only
/// `.gitignore` files in the target and below.
fn configure_gitignore(builder: &mut WalkBuilder, target: &Path, enabled: bool) {
    let in_repo = enabled && repository_root(target).is_some();
    builder
        .git_ignore(enabled)
        .git_exclude(in_repo)
        .parents(in_repo)
        .require_git(in_repo);
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

/// The most brace alternatives one glob target may expand to.
const MAX_ALTERNATIVES: usize = 1024;

/// Match a non-existent target as a glob, relative to the working directory or
/// absolute. Only files match; results are sorted and unique.
///
/// Brace groups are expanded into brace-free patterns first, so each one gets its
/// own depth bound and directory pruning.
fn expand_glob(pattern: &str) -> Result<Vec<PathBuf>, DiscoverError> {
    // `./docs/*.md` and `docs/*.md` are the same pattern; candidates below are
    // normalised the same way.
    let normalised: PathBuf = Path::new(pattern)
        .components()
        .filter(|c| *c != Component::CurDir)
        .collect();
    let normalised = normalised.to_string_lossy();
    let alternatives =
        syntax::expand_braces(&normalised, MAX_ALTERNATIVES, syntax::BACKSLASH_ESCAPES)
            .ok_or_else(|| DiscoverError::TooManyAlternatives {
                pattern: pattern.to_owned(),
                limit: MAX_ALTERNATIVES,
            })?;
    let mut matches = BTreeSet::new();
    for alternative in &alternatives {
        matches.extend(glob_files(alternative, pattern)?);
    }
    if matches.is_empty() {
        return Err(DiscoverError::NoSuchTarget(pattern.to_owned()));
    }
    Ok(matches.into_iter().collect())
}

/// Files matching one brace-free `pattern`. `target` is the pattern as typed.
fn glob_files(pattern: &str, target: &str) -> Result<Vec<PathBuf>, DiscoverError> {
    let invalid = |source| DiscoverError::InvalidGlob {
        pattern: target.to_owned(),
        source,
    };
    let compile = |text: &str| {
        GlobBuilder::new(text)
            .literal_separator(true)
            .build()
            .map(|g| g.compile_matcher())
            .map_err(invalid)
    };
    compile(pattern)?; // reject a malformed pattern before touching the disk

    // Walk from the longest leading run of components without glob syntax. The
    // last component is never part of it: it names what to match.
    let mut base = PathBuf::new();
    let components: Vec<_> = Path::new(pattern).components().collect();
    for component in &components[..components.len().saturating_sub(1)] {
        let text = component.as_os_str().to_string_lossy();
        if text.contains(['*', '?', '[', '{']) {
            break;
        }
        base.push(component);
    }
    let base_len = base.components().count();
    // Resolve the base once, as a direct target would be, so every match is
    // emitted under the resolved directory (`link/..` is the link target's parent).
    let base = syntax::resolve_target(&base);
    if !base.is_dir() {
        return Ok(Vec::new());
    }

    // Bound the walk by the pattern: no recursion without `**`, and directories
    // that cannot match their component are never read.
    let rest: Vec<String> = Path::new(pattern)
        .components()
        .skip(base_len)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    // Components before the first `**` each fix one directory level; from `**`
    // on, any depth can match and nothing is pruned.
    let first_recursive = rest.iter().position(|c| c.contains("**"));
    let prefix = rest[..first_recursive.unwrap_or(rest.len())]
        .iter()
        .map(|c| compile(c))
        .collect::<Result<Vec<_>, _>>()?;
    let recursive = first_recursive.is_some();
    // Matching uses the part below the base, so the resolved base needs no escaping.
    let below = compile(&rest.join("/"))?;
    let mut builder = WalkBuilder::new(&base);
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .max_depth((!recursive).then_some(rest.len()))
        .filter_entry(move |entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            match entry.depth().checked_sub(1).and_then(|d| prefix.get(d)) {
                Some(component) if is_dir => component.is_match(entry.file_name()),
                _ => true,
            }
        });
    let mut matches = Vec::new();
    for entry in builder.build() {
        let entry = entry.map_err(|source| DiscoverError::Walk {
            path: base.clone(),
            source,
        })?;
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(&base) else {
            continue;
        };
        if below.is_match(relative) && path.is_file() {
            matches.push(
                path.components()
                    .filter(|c| *c != Component::CurDir)
                    .collect(),
            );
        }
    }
    Ok(matches)
}

#[cfg(test)]
mod tests;
