//! Planning, previewing and applying a migration.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use similar::TextDiff;
use toml_edit::DocumentMut;

use crate::config::{key_lines, migrate_config};
use crate::{Dropped, Error, Options, RenameTable, rewrite_directives, rewrite_steering};

/// Where `apply` keeps the files it replaced, relative to the project root.
pub const BACKUP_DIR: &str = ".slopvac-migrate-backup";

/// What a change does to its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Create or overwrite the file.
    Write,
    /// Delete the file.
    Delete,
}

/// One file edit, with the path relative to the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The file.
    pub path: PathBuf,
    /// Write or delete.
    pub kind: ChangeKind,
    /// The text on disk now, if the file exists.
    pub old: Option<String>,
    /// The text to write. `None` for a delete.
    pub new: Option<String>,
}

/// Every edit a migration makes, plus what it found worth saying.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// File edits in the order `apply` performs them.
    pub changes: Vec<Change>,
    /// Config changes that are more than a plain copy.
    pub notes: Vec<String>,
    /// Things the user should check, such as an unknown directive id.
    pub warnings: Vec<String>,
}

impl Plan {
    /// True when nothing needs to change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The new text for each written file, for linting the migrated tree.
    #[must_use]
    pub fn overlay(&self, root: &Path) -> Vec<(PathBuf, String)> {
        self.changes
            .iter()
            .filter_map(|c| Some((root.join(&c.path), c.new.clone()?)))
            .collect()
    }
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".venv",
    "venv",
    BACKUP_DIR,
];

/// Plan the migration of the project under `root`.
///
/// Reads only. Every unrepresentable setting across all files comes back together in
/// [`Error::Dropped`].
pub fn plan(root: &Path, renames: &RenameTable, options: Options) -> Result<Plan, Error> {
    let mut plan = Plan::default();
    let mut dropped: Vec<Dropped> = Vec::new();
    let mut dirs = Vec::new();
    let mut docs = Vec::new();
    walk(root, &mut dirs, &mut docs)?;
    for dir in dirs {
        plan_dir(root, &dir, renames, options, &mut plan, &mut dropped)?;
    }
    for doc in docs {
        plan_document(root, &doc, renames, &mut plan);
    }
    if dropped.is_empty() {
        Ok(plan)
    } else {
        Err(Error::Dropped(dropped))
    }
}

fn walk(dir: &Path, dirs: &mut Vec<PathBuf>, docs: &mut Vec<PathBuf>) -> Result<(), Error> {
    dirs.push(dir.to_owned());
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| Error::io(dir, e))?
        .collect::<Result<_, _>>()
        .map_err(|e| Error::io(dir, e))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = entry.file_type().map_err(|e| Error::io(&path, e))?;
        if kind.is_dir() {
            if !SKIP_DIRS.contains(&name.as_ref()) {
                walk(&path, dirs, docs)?;
            }
        } else if kind.is_file() && name.ends_with(".md") {
            docs.push(path);
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|e| Error::io(path, e))
}

fn rel(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_owned()
}

fn plan_dir(
    root: &Path,
    dir: &Path,
    renames: &RenameTable,
    options: Options,
    plan: &mut Plan,
    dropped: &mut Vec<Dropped>,
) -> Result<(), Error> {
    let main_cfg = dir.join("slopvac.toml");
    let dot = dir.join(".slopvac.toml");
    let pyproject = dir.join("pyproject.toml");
    let py_text = if pyproject.is_file() {
        Some(read(&pyproject)?)
    } else {
        None
    };
    let py_table = py_text.as_deref().and_then(extract_tool_table);
    let sources = usize::from(main_cfg.is_file())
        + usize::from(dot.is_file())
        + usize::from(py_table.is_some());
    if sources > 1 {
        return Err(Error::Conflict(format!(
            "{}: more than one 2.x config source (slopvac.toml, .slopvac.toml, [tool.slopvac]); \
             2.x read only one of them. Remove the ones that are unused, then rerun",
            dir.display()
        )));
    }
    let (source, origin, old) = if main_cfg.is_file() {
        (main_cfg.clone(), Origin::InPlace, read(&main_cfg)?)
    } else if dot.is_file() {
        (dot.clone(), Origin::Dotfile, read(&dot)?)
    } else if let (Some((extracted, _)), true) = (&py_table, py_text.is_some()) {
        (pyproject.clone(), Origin::Pyproject, extracted.clone())
    } else {
        return Ok(());
    };
    let label = rel(root, &source).display().to_string();
    let rewrite = match migrate_config(&old, &label, renames, options) {
        Ok(rw) => rw,
        Err(Error::AlreadyMigrated { .. }) => return Ok(()),
        Err(Error::Dropped(d)) => {
            let lines = py_text
                .as_deref()
                .filter(|_| origin == Origin::Pyproject)
                .and_then(|t| key_lines(t).ok());
            dropped.extend(d.into_iter().map(|mut x| {
                if let Some(l) = lines
                    .as_ref()
                    .and_then(|m| m.get(&format!("tool.slopvac.{}", x.key)))
                {
                    x.line = *l;
                }
                x
            }));
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    plan.notes
        .extend(rewrite.notes.into_iter().map(|n| format!("{label}: {n}")));
    let target = rel(root, &main_cfg);
    match origin {
        Origin::InPlace => plan.changes.push(Change {
            path: target,
            kind: ChangeKind::Write,
            old: Some(old),
            new: Some(rewrite.text),
        }),
        Origin::Dotfile => {
            plan.notes.push(format!("{label}: renamed to slopvac.toml"));
            plan.changes.push(Change {
                path: target,
                kind: ChangeKind::Write,
                old: None,
                new: Some(rewrite.text),
            });
            plan.changes.push(Change {
                path: rel(root, &dot),
                kind: ChangeKind::Delete,
                old: Some(old),
                new: None,
            });
        }
        Origin::Pyproject => {
            plan.notes
                .push(format!("{label}: moved [tool.slopvac] to slopvac.toml"));
            plan.changes.push(Change {
                path: target,
                kind: ChangeKind::Write,
                old: None,
                new: Some(rewrite.text),
            });
            plan.changes.push(Change {
                path: rel(root, &pyproject),
                kind: ChangeKind::Write,
                old: py_text,
                new: py_table.map(|(_, stripped)| stripped),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    InPlace,
    Dotfile,
    Pyproject,
}

/// The `[tool.slopvac]` table as a standalone document, and `pyproject.toml` text
/// without it. Unrelated content and comments in `pyproject.toml` stay as they are.
fn extract_tool_table(text: &str) -> Option<(String, String)> {
    let mut doc: DocumentMut = text.parse().ok()?;
    let table = doc.get("tool")?.get("slopvac")?.as_table()?.clone();
    let tool = doc.get_mut("tool")?.as_table_like_mut()?;
    tool.remove("slopvac");
    if tool.is_empty() {
        doc.remove("tool");
    }
    let mut standalone = DocumentMut::new();
    *standalone.as_table_mut() = table;
    Some((standalone.to_string(), doc.to_string()))
}

fn plan_document(root: &Path, path: &Path, renames: &RenameTable, plan: &mut Plan) {
    let Ok(old) = fs::read_to_string(path) else {
        return;
    };
    let label = rel(root, path);
    let (mut text, unknown) = rewrite_directives(&old, renames);
    for id in unknown {
        plan.warnings.push(format!(
            "{}: directive rule id `{id}` has no 3.0 name; left unchanged",
            label.display()
        ));
    }
    match rewrite_steering(&text) {
        Ok(Some(new)) => text = new,
        Ok(None) => {}
        Err(msg) => plan.warnings.push(format!("{}: {msg}", label.display())),
    }
    if text != old {
        plan.changes.push(Change {
            path: label,
            kind: ChangeKind::Write,
            old: Some(old),
            new: Some(text),
        });
    }
}

/// A unified diff of every change in the plan.
#[must_use]
pub fn render_diff(plan: &Plan) -> String {
    let mut out = String::new();
    for c in &plan.changes {
        let name = c.path.display();
        let old = c.old.as_deref().unwrap_or("");
        let new = c.new.as_deref().unwrap_or("");
        let a = if c.old.is_some() {
            format!("a/{name}")
        } else {
            "/dev/null".to_owned()
        };
        let b = if c.new.is_some() {
            format!("b/{name}")
        } else {
            "/dev/null".to_owned()
        };
        let diff = TextDiff::from_lines(old, new);
        let _ = write!(out, "{}", diff.unified_diff().header(&a, &b));
    }
    out
}

/// Write the plan to disk. Replaced and deleted files are copied to [`BACKUP_DIR`]
/// first, with a manifest that [`restore`] reads.
pub fn apply(root: &Path, plan: &Plan) -> Result<(), Error> {
    let backup = root.join(BACKUP_DIR);
    fs::create_dir_all(&backup).map_err(|e| Error::io(&backup, e))?;
    let mut manifest = String::new();
    for c in &plan.changes {
        let full = root.join(&c.path);
        if full.is_file() {
            let dest = backup.join(&c.path);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            fs::copy(&full, &dest).map_err(|e| Error::io(&full, e))?;
            let _ = writeln!(manifest, "M {}", c.path.display());
        } else {
            let _ = writeln!(manifest, "C {}", c.path.display());
        }
    }
    let mpath = backup.join("manifest.txt");
    fs::write(&mpath, manifest).map_err(|e| Error::io(&mpath, e))?;
    for c in &plan.changes {
        let full = root.join(&c.path);
        match (&c.kind, &c.new) {
            (ChangeKind::Write, Some(text)) => {
                fs::write(&full, text).map_err(|e| Error::io(&full, e))?;
            }
            _ => fs::remove_file(&full).map_err(|e| Error::io(&full, e))?,
        }
    }
    Ok(())
}

/// Undo the last [`apply`]: put backed-up files back and remove files it created.
/// Returns the number of files touched.
pub fn restore(root: &Path) -> Result<usize, Error> {
    let backup = root.join(BACKUP_DIR);
    let manifest = read(&backup.join("manifest.txt"))?;
    let mut count = 0;
    for line in manifest.lines() {
        let Some((kind, name)) = line.split_once(' ') else {
            continue;
        };
        let full = root.join(name);
        match kind {
            "M" => {
                fs::copy(backup.join(name), &full).map_err(|e| Error::io(&full, e))?;
            }
            "C" if full.is_file() => {
                fs::remove_file(&full).map_err(|e| Error::io(&full, e))?;
            }
            "C" => {}
            _ => continue,
        }
        count += 1;
    }
    fs::remove_dir_all(&backup).map_err(|e| Error::io(&backup, e))?;
    Ok(count)
}
