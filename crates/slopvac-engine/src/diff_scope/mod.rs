//! Changed-line scope from git: which lines of which files a branch or the work tree
//! changed.
//!
//! Git supplies the changed line intervals. The engine still inspects each whole
//! document, and [`retain_in_scope`] then drops the findings outside those intervals.
//! This is the 2.x `--diff-base` and `--diff-working-tree` behaviour (`diff_scope.py`).
//!
//! - [`DiffScope::Base`] compares `<base>...HEAD`: what the current branch added since it
//!   left `<base>`.
//! - [`DiffScope::WorkingTree`] compares `HEAD` to the index and the work tree together,
//!   so staged and unstaged edits are both in scope, and adds every untracked,
//!   non-ignored file as a whole-file change.
//!
//! [`changed_lines`] runs the scope once and returns a [`ChangedLines`]. A caller looks a
//! file up with [`ChangedLines::ranges_for`] and filters its findings with
//! [`retain_in_scope`]. A fixer asks [`finding_fixable`] so a replacement never touches
//! an unchanged line (`fix_file`'s `changed_lines` argument takes
//! [`ChangedLines::ranges_for`] output).
//!
//! Rules carried over from 2.x:
//!
//! - Pure deletions leave no range. Renames carry the new name's added lines, and a
//!   rename without edits has none. Binary files have none. Submodules are ignored.
//! - A symlink is never followed. One that git reports as changed is recorded, and only
//!   a lookup for that path fails with [`DiffError::Symlink`] (2.x #253): a symlink added
//!   outside the lint targets does not fail the run. A deleted symlink is not recorded.
//! - A finding is kept when any line of its full span touches a changed range. A finding
//!   without a source range is dropped, as the 2.x metric findings were.
//!
//! Git runs as a hardened subprocess with an output cap and a deadline; see the `git`
//! module's notes for the trust model. The patch names lines by LF only, so
//! [`ChangedLines::ranges_for`] translates them to the line numbers findings use.

mod git;
mod lines;
mod patch;
mod read;

pub use git::{Limits, Options};

use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;
use std::path::{Component, Path, PathBuf};

use slopvac_model::Finding;
use thiserror::Error;

/// The last line number a whole-file range reaches.
const WHOLE_FILE_END: u32 = u32::MAX;

/// Which change set to scope a lint to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffScope {
    /// `<base>...HEAD`: the lines the current branch added since the merge base.
    Base(String),
    /// `HEAD` against the index and work tree, plus untracked files.
    WorkingTree,
}

/// Why a change set could not be determined.
#[derive(Debug, Error)]
pub enum DiffError {
    /// The `git` program did not start.
    #[error("{}", crate::messages::git_spawn(.0))]
    Spawn(#[source] std::io::Error),
    /// The directory is not inside a git work tree.
    #[error("{}", crate::messages::git_repository(.0))]
    NotRepository(String),
    /// Git ran and failed. The text is its standard error.
    #[error("{}", crate::messages::git_error(.0))]
    Git(String),
    /// The base is not a commit, or is not a usable revision name.
    #[error("{}", crate::messages::git_revision(.0))]
    InvalidBase(String),
    /// Git printed a patch this parser does not accept.
    #[error("{}", crate::messages::git_malformed(.0))]
    Malformed(String),
    /// Git needs an object this clone does not have, and fetching is off. Fetch it, then
    /// run again. The check is incomplete.
    #[error("{}", crate::messages::git_object_missing(.0))]
    MissingObject(String),
    /// Git printed more than the output cap. The check is incomplete.
    #[error("{}", crate::messages::EngineText::GitOutputLimit { limit: *.0 })]
    OutputTooLarge(usize),
    /// Git ran past the deadline and was stopped. The check is incomplete.
    #[error("{}", crate::messages::git_timeout(.0))]
    TimedOut(std::time::Duration),
    /// A changed file is larger than [`Limits::max_file`], so it was not read.
    #[error("{}", crate::messages::changed_file_too_large(.path, *.limit))]
    FileTooLarge {
        /// The file.
        path: PathBuf,
        /// The cap in bytes.
        limit: u64,
    },
    /// A changed file is no longer a regular file (a FIFO or a device now).
    #[error("{}", crate::messages::git_not_regular(.0))]
    NotRegularFile(PathBuf),
    /// A changed path is a symlink.
    #[error("{}", crate::messages::git_symlink(.0))]
    Symlink(PathBuf),
    /// A changed path is absolute or climbs out of the repository.
    #[error("{}", crate::messages::git_escape(.0))]
    Escape(PathBuf),
}

/// The changed lines of one repository, resolved once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedLines {
    root: PathBuf,
    max_file: u64,
    files: BTreeMap<PathBuf, Vec<RangeInclusive<u32>>>,
    symlinks: BTreeSet<PathBuf>,
}

impl ChangedLines {
    /// The canonical work tree root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The changed, non-symlink files with their ranges, keyed by absolute path under
    /// [`Self::root`]. Ranges are 1-based, inclusive, sorted and merged, in git's LF-only
    /// line numbers; use [`Self::ranges_for`] for the numbers findings carry.
    #[must_use]
    pub fn files(&self) -> &BTreeMap<PathBuf, Vec<RangeInclusive<u32>>> {
        &self.files
    }

    /// The changed lines of `path`, in the line numbers findings use. Empty when git
    /// reports no added line for it.
    ///
    /// Git counts lines by LF only, while findings count LF, CRLF and a lone CR as line
    /// ends. This reads the file once, in chunks and without keeping it, and translates
    /// the ranges; a git line that spans several source lines maps to all of them. A
    /// file that is absent or unreadable keeps git's numbers.
    ///
    /// The open does not follow a symlink and does not block on a FIFO, and the file is
    /// read up to [`Limits::max_file`] bytes. A whole-file range is returned without a
    /// read, and [`Self::is_changed`] and [`Self::is_whole_file`] never read.
    ///
    /// `path` may be relative to the current directory. Its directory is resolved but the
    /// final name is not, so a symlink is seen as a symlink.
    ///
    /// # Errors
    ///
    /// [`DiffError::Symlink`] when `path` is a symlink that git reports as changed or one
    /// that replaced the file since; [`DiffError::NotRegularFile`] for a FIFO or device;
    /// [`DiffError::FileTooLarge`] over the cap.
    pub fn ranges_for(&self, path: &Path) -> Result<Vec<RangeInclusive<u32>>, DiffError> {
        let (key, ranges) = self.raw(path)?;
        // Nothing to translate: no range, or the whole file.
        if ranges.is_empty() || ranges.iter().any(|r| *r.end() == WHOLE_FILE_END) {
            return Ok(ranges.to_vec());
        }
        let Some(mut file) = read::open_regular(&key, self.max_file)? else {
            return Ok(ranges.to_vec());
        };
        // One byte past the cap catches a file that grew after the size check.
        let mut bounded = std::io::Read::take(&mut file, self.max_file.saturating_add(1));
        match lines::to_source_lines(&mut bounded, ranges) {
            Ok((_, read)) if read > self.max_file => Err(DiffError::FileTooLarge {
                path: key,
                limit: self.max_file,
            }),
            Ok((mapped, _)) => Ok(mapped),
            Err(_) => Ok(ranges.to_vec()),
        }
    }

    /// The lookup key for `path` and git's ranges for it, without reading the file.
    fn raw(&self, path: &Path) -> Result<(PathBuf, &[RangeInclusive<u32>]), DiffError> {
        let key = locate(path);
        if self.symlinks.contains(&key) {
            return Err(DiffError::Symlink(key));
        }
        let ranges = self.files.get(&key).map_or(&[][..], Vec::as_slice);
        Ok((key, ranges))
    }

    /// Whether `path` is changed in every line, as an untracked file is.
    ///
    /// # Errors
    ///
    /// [`DiffError::Symlink`] as for [`Self::ranges_for`].
    pub fn is_whole_file(&self, path: &Path) -> Result<bool, DiffError> {
        Ok(self
            .raw(path)?
            .1
            .iter()
            .any(|r| *r.start() == 1 && *r.end() == WHOLE_FILE_END))
    }

    /// Whether `path` has at least one changed line, for narrowing lint targets.
    ///
    /// # Errors
    ///
    /// [`DiffError::Symlink`] as for [`Self::ranges_for`].
    pub fn is_changed(&self, path: &Path) -> Result<bool, DiffError> {
        Ok(!self.raw(path)?.1.is_empty())
    }
}

/// Resolve the changed lines for `scope` in the repository that contains `dir`, with the
/// default [`Limits`].
///
/// # Errors
///
/// A [`DiffError`] when git is missing or fails, `dir` is not in a work tree, the base
/// is not a commit, an object is missing, a limit is hit, or the patch is malformed. The
/// scope is never partial.
pub fn changed_lines(dir: &Path, scope: &DiffScope) -> Result<ChangedLines, DiffError> {
    changed_lines_with(dir, scope, &Options::default())
}

/// [`changed_lines`] with explicit limits and git program.
///
/// # Errors
///
/// As [`changed_lines`].
pub fn changed_lines_with(
    dir: &Path,
    scope: &DiffScope,
    options: &Options,
) -> Result<ChangedLines, DiffError> {
    let toplevel =
        git::run(dir, &["rev-parse", "--show-toplevel"], options).map_err(|e| match e {
            DiffError::Git(detail) if detail.is_empty() => {
                DiffError::NotRepository("not inside a Git repository".to_owned())
            }
            DiffError::Git(detail) => DiffError::NotRepository(detail),
            other => other,
        })?;
    let toplevel = patch::to_path(trim_newline(&toplevel))?;
    let root = std::fs::canonicalize(&toplevel).map_err(|e| {
        DiffError::Git(
            crate::messages::EngineText::GitResolveRoot {
                reason: e.to_string(),
            }
            .to_string(),
        )
    })?;

    let range;
    let mut diff_args: Vec<&str> = git::DIFF_ARGS.to_vec();
    match scope {
        DiffScope::Base(base) => {
            range = format!("{}...HEAD", resolve_base(&root, base, options)?);
            diff_args.push(&range);
        }
        DiffScope::WorkingTree => diff_args.push("HEAD"),
    }
    diff_args.push("--");
    let parsed = patch::parse(&git::run(&root, &diff_args, options)?)?;

    let mut changed = ChangedLines {
        root,
        max_file: options.limits.max_file,
        files: BTreeMap::new(),
        symlinks: BTreeSet::new(),
    };
    for (relative, ranges) in parsed {
        changed.insert(&relative, ranges)?;
    }
    if matches!(scope, DiffScope::WorkingTree) {
        let listing = git::run(
            &changed.root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
            options,
        )?;
        for entry in listing.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            // A nested repository is listed as `dir/`; it is not a file.
            if entry.ends_with(b"/") {
                continue;
            }
            changed.insert(
                &patch::to_path(entry)?,
                std::iter::once(1..=WHOLE_FILE_END).collect(),
            )?;
        }
    }
    Ok(changed)
}

impl ChangedLines {
    /// Record `relative` under the root, or note it as a symlink.
    fn insert(
        &mut self,
        relative: &Path,
        ranges: Vec<RangeInclusive<u32>>,
    ) -> Result<(), DiffError> {
        if !relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(DiffError::Escape(relative.to_path_buf()));
        }
        let absolute = self.root.join(relative);
        if has_symlink_component(&self.root, relative) {
            self.symlinks.insert(absolute);
        } else {
            self.files.insert(absolute, ranges);
        }
        Ok(())
    }
}

/// Turn `base` into a full commit id, so only an id reaches `git diff`.
fn resolve_base(root: &Path, base: &str, options: &Options) -> Result<String, DiffError> {
    // A leading dash would read as an option; a revision name cannot start with one.
    if base.is_empty() || base.starts_with('-') {
        return Err(DiffError::InvalidBase(base.to_owned()));
    }
    let spec = format!("{base}^{{commit}}");
    let out = git::run(root, &["rev-parse", "--verify", "--quiet", &spec], options).map_err(
        |e| match e {
            DiffError::Git(_) => DiffError::InvalidBase(base.to_owned()),
            other => other,
        },
    )?;
    let id = String::from_utf8_lossy(trim_newline(&out)).into_owned();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(DiffError::InvalidBase(base.to_owned()));
    }
    Ok(id)
}

fn trim_newline(bytes: &[u8]) -> &[u8] {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    bytes.strip_suffix(b"\r").unwrap_or(bytes)
}

/// Whether the file or any directory leading to it is a symlink. A missing component is
/// not a symlink.
fn has_symlink_component(root: &Path, relative: &Path) -> bool {
    let mut current = root.to_path_buf();
    relative.components().any(|part| {
        current.push(part);
        std::fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink())
    })
}

/// Make a lookup key: the directory resolved, the final name kept.
fn locate(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    };
    match (absolute.parent(), absolute.file_name()) {
        (Some(parent), Some(name)) => {
            std::fs::canonicalize(parent).map_or_else(|_| absolute.clone(), |dir| dir.join(name))
        }
        _ => absolute,
    }
}

/// The first and last line of a finding's span, or `None` for a finding with no source
/// range (a metric finding) or no matched text.
fn span_lines(finding: &Finding) -> Option<(u32, u32)> {
    let extent = finding.extent.as_ref()?;
    if finding.matched_text.is_empty() {
        return None;
    }
    let line = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Some((line(finding.start.line()), line(extent.end.line())))
}

/// Whether any line of the finding's full span touches a changed range.
#[must_use]
pub fn finding_in_scope(finding: &Finding, ranges: &[RangeInclusive<u32>]) -> bool {
    span_lines(finding).is_some_and(|(first, last)| {
        ranges
            .iter()
            .any(|r| first <= *r.end() && last >= *r.start())
    })
}

/// Whether replacing the finding cannot touch an unchanged line: it is in scope, has a
/// fix, and every line of its span is changed.
#[must_use]
pub fn finding_fixable(finding: &Finding, ranges: &[RangeInclusive<u32>]) -> bool {
    if finding.fix.is_none() || !finding_in_scope(finding, ranges) {
        return false;
    }
    span_lines(finding).is_some_and(|(first, last)| {
        (first..=last).all(|line| ranges.iter().any(|r| r.contains(&line)))
    })
}

/// Drop every finding that [`finding_in_scope`] rejects.
pub fn retain_in_scope(findings: &mut Vec<Finding>, ranges: &[RangeInclusive<u32>]) {
    findings.retain(|finding| finding_in_scope(finding, ranges));
}
