//! The one directory walk behind both the planner and the linter.

use std::fs;
use std::path::{Path, PathBuf};

use slopvac_engine::discover::{self, FileKind};

use crate::capped::Capped;
use crate::limits::Budget;
use crate::{BACKUP_DIR, Error};

/// A directory the walk entered, with the 2.x config files it holds.
#[derive(Debug)]
pub(crate) struct Dir {
    pub(crate) path: PathBuf,
    pub(crate) main: bool,
    pub(crate) dot: bool,
    pub(crate) pyproject: bool,
}

/// What a walk found.
#[derive(Debug, Default)]
pub(crate) struct Tree {
    /// Directories in walk order.
    pub(crate) dirs: Vec<Dir>,
    /// Markdown and MDX documents in walk order.
    pub(crate) docs: Vec<(PathBuf, FileKind)>,
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Whether a file with this name is one migrate would look at.
fn is_candidate(name: &str) -> bool {
    matches!(name, "slopvac.toml" | ".slopvac.toml" | "pyproject.toml")
        || matches!(
            FileKind::from_file_name(name),
            Some(
                FileKind::Markdown
                    | FileKind::Mdx
                    | FileKind::Html
                    | FileKind::Rst
                    | FileKind::Text
            )
        )
}

/// Walk `root`. Every entry is charged to `budget` before it is stored. Symbolic links are
/// never followed. A link, a special file or an unsupported prose file that migrate could
/// have looked at goes to `warnings`.
pub(crate) fn discover(
    root: &Path,
    budget: &mut Budget,
    warnings: &mut Capped,
) -> Result<Tree, Error> {
    let mut tree = Tree::default();
    walk(root, root, budget, warnings, &mut tree)?;
    Ok(tree)
}

fn walk(
    root: &Path,
    dir: &Path,
    budget: &mut Budget,
    warnings: &mut Capped,
    tree: &mut Tree,
) -> Result<(), Error> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        budget.charge_entry(&entry.path())?;
        entries.push(entry);
    }
    entries.sort_by_key(fs::DirEntry::file_name);
    let mut here = Dir {
        path: dir.to_owned(),
        main: false,
        dot: false,
        pyproject: false,
    };
    let mut subdirs = Vec::new();
    let mut docs = Vec::new();
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = entry.file_type().map_err(|e| Error::io(&path, e))?;
        if kind.is_dir() {
            if name != BACKUP_DIR && !discover::SKIP_DIRS.contains(&name.as_ref()) {
                subdirs.push(path);
            }
        } else if kind.is_symlink() {
            let to_dir = fs::metadata(&path).is_ok_and(|m| m.is_dir());
            if to_dir || is_candidate(&name) {
                warnings.push(format!(
                    "{}: not checked: symbolic link (links are not followed)",
                    rel(root, &path)
                ));
            }
        } else if kind.is_file() {
            match name.as_ref() {
                "slopvac.toml" => here.main = true,
                ".slopvac.toml" => here.dot = true,
                "pyproject.toml" => here.pyproject = true,
                _ => {}
            }
            match FileKind::from_file_name(&name) {
                Some(kind @ (FileKind::Markdown | FileKind::Mdx)) => docs.push((path, kind)),
                Some(kind @ (FileKind::Html | FileKind::Rst | FileKind::Text)) => {
                    warnings.push(format!(
                        "{}: not checked: migrate reads Markdown and MDX only, not {kind:?} files",
                        rel(root, &path)
                    ));
                }
                _ => {}
            }
        } else if is_candidate(&name) {
            warnings.push(format!(
                "{}: not checked: not a regular file",
                rel(root, &path)
            ));
        }
    }
    tree.dirs.push(here);
    // Documents of this directory come before those of its sub-directories, each group
    // in name order.
    tree.docs.extend(docs);
    for sub in subdirs {
        walk(root, &sub, budget, warnings, tree)?;
    }
    Ok(())
}
