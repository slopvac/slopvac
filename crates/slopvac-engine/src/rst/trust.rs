//! Find an interpreter that is safe to run on untrusted documents.
//!
//! A repository can put an executable named `python3` where a `PATH` search finds it: an
//! activated virtual environment, `node_modules/.bin`, a directory a project tool adds. So
//! discovery trusts only directories the user controls:
//!
//! - A relative or empty `PATH` entry is skipped.
//! - An entry that resolves inside a project root (the repository being linted, or a
//!   directory the user asked to lint) is skipped. So is an interpreter whose symlink
//!   resolves there.
//! - A world-writable directory is skipped.
//!
//! The user can name an interpreter in `SLOPVAC_RST_CONVERTER`. It needs an absolute path
//! outside every project root. Project files never set it.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Interpreter names, in order. The 2.x release read `rst2html`; 3.0 runs docutils through
/// the interpreter that has it.
pub const INTERPRETER_NAMES: [&str; 2] = ["python3", "python"];

/// The environment variable that names an interpreter.
pub const OVERRIDE_VARIABLE: &str = "SLOPVAC_RST_CONVERTER";

/// Why a candidate was not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The `PATH` entry is empty.
    EmptyEntry,
    /// The `PATH` entry is a relative path.
    RelativeEntry,
    /// The directory or the interpreter resolves inside a project root.
    InsideProject(PathBuf),
    /// Anyone can write to the directory.
    WorldWritable,
    /// The interpreter could not import docutils 0.21 or later.
    NoDocutils,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyEntry => f.write_str("empty PATH entry"),
            Self::RelativeEntry => f.write_str("relative PATH entry"),
            Self::InsideProject(root) => write!(f, "inside the project {}", root.display()),
            Self::WorldWritable => f.write_str("world-writable directory"),
            Self::NoDocutils => f.write_str("docutils 0.21 or later is not importable"),
        }
    }
}

/// A `PATH` entry or interpreter that was not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The entry or interpreter.
    pub path: PathBuf,
    /// Why.
    pub reason: SkipReason,
}

/// What discovery found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Discovery {
    /// The interpreter to run, as found on the search path.
    pub interpreter: Option<PathBuf>,
    /// Everything it passed over.
    pub skipped: Vec<Skipped>,
}

/// Why an explicit interpreter is not usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideError {
    /// The path is not absolute.
    NotAbsolute,
    /// The path is not an executable file.
    NotExecutable,
    /// The interpreter resolves inside a project root.
    InsideProject(PathBuf),
    /// The path exists as an executable but cannot be resolved to its real location.
    Unresolvable(String),
}

impl std::fmt::Display for OverrideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAbsolute => f.write_str("it is not an absolute path"),
            Self::NotExecutable => f.write_str("it is not an executable file"),
            Self::InsideProject(root) => write!(f, "it is inside the project {}", root.display()),
            Self::Unresolvable(reason) => write!(f, "it cannot be resolved: {reason}"),
        }
    }
}

/// Canonical forms of the project roots. A root that does not exist is dropped.
fn canonical_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .collect()
}

fn inside(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots.iter().find(|r| path.starts_with(r)).cloned()
}

/// Search `search_path` for the first interpreter the user can trust.
///
/// `usable` decides whether a candidate can import docutils; it is called at most once per
/// candidate, in order, and the first `true` wins.
pub fn discover(
    search_path: &OsStr,
    project_roots: &[PathBuf],
    mut usable: impl FnMut(&Path) -> bool,
) -> Discovery {
    let roots = canonical_roots(project_roots);
    let mut found = Discovery::default();
    let mut trusted: Vec<PathBuf> = Vec::new();
    for entry in std::env::split_paths(search_path) {
        if entry.as_os_str().is_empty() {
            found.skipped.push(Skipped {
                path: entry,
                reason: SkipReason::EmptyEntry,
            });
            continue;
        }
        if !entry.is_absolute() {
            found.skipped.push(Skipped {
                path: entry,
                reason: SkipReason::RelativeEntry,
            });
            continue;
        }
        let Ok(canonical) = std::fs::canonicalize(&entry) else {
            continue; // Not a directory that exists: nothing to report.
        };
        if let Some(root) = inside(&canonical, &roots) {
            found.skipped.push(Skipped {
                path: entry,
                reason: SkipReason::InsideProject(root),
            });
            continue;
        }
        if world_writable(&canonical) {
            found.skipped.push(Skipped {
                path: entry,
                reason: SkipReason::WorldWritable,
            });
            continue;
        }
        trusted.push(entry);
    }
    for name in INTERPRETER_NAMES {
        for dir in &trusted {
            let Some(candidate) = executable_in(dir, name) else {
                continue;
            };
            // A symlink can lead into the project even from a system directory.
            if let Some(root) = std::fs::canonicalize(&candidate)
                .ok()
                .and_then(|real| inside(&real, &roots))
            {
                found.skipped.push(Skipped {
                    path: candidate,
                    reason: SkipReason::InsideProject(root),
                });
                continue;
            }
            if usable(&candidate) {
                found.interpreter = Some(candidate);
                return found;
            }
            found.skipped.push(Skipped {
                path: candidate,
                reason: SkipReason::NoDocutils,
            });
        }
    }
    found
}

/// Check an interpreter the user named.
///
/// # Errors
/// Returns the reason the path cannot be trusted.
pub fn check_override(path: &Path, project_roots: &[PathBuf]) -> Result<PathBuf, OverrideError> {
    if !path.is_absolute() {
        return Err(OverrideError::NotAbsolute);
    }
    if !is_executable(path) {
        return Err(OverrideError::NotExecutable);
    }
    let roots = canonical_roots(project_roots);
    let real =
        std::fs::canonicalize(path).map_err(|e| OverrideError::Unresolvable(e.to_string()))?;
    if let Some(root) = inside(&real, &roots) {
        return Err(OverrideError::InsideProject(root));
    }
    Ok(path.to_owned())
}

/// `dir/name` when it is an executable file.
fn executable_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) { &["", ".exe"] } else { &[""] };
    extensions.iter().find_map(|extension| {
        let path = dir.join(format!("{name}{extension}"));
        is_executable(&path).then_some(path)
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

#[cfg(unix)]
fn world_writable(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(dir).is_ok_and(|m| m.permissions().mode() & 0o002 != 0)
}

#[cfg(not(unix))]
fn world_writable(_dir: &Path) -> bool {
    false
}
