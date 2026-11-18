//! The [`Linter`] over `slopvac-engine`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use slopvac_engine::discover::{self, FileKind};
use slopvac_engine::{Engine, Input, Settings};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString, Notice, NoticeLevel};
use slopvac_rules::RuleSet;

use crate::{BACKUP_DIR, Error, Finding, Linter, Phase, RenameTable, rewrite_directives};

/// Lints the Markdown and MDX documents of a project with the engine.
///
/// Both sides run the 3.0 engine, so the comparison shows how the 3.0 engine sees
/// the files before and after migration. It does not run 2.x. `Before` reads every
/// document from disk and reads each `slopvac-allow` id through the rename table, in
/// memory only: a directive that 2.x accepted then suppresses on that side as it did
/// in 2.x, and does not count as an invalid suppression. A directive 2.x rejected,
/// such as a comma list or an unknown id, stays as written. `After` reads a document
/// from the overlay when the plan rewrites it. Files under [`BACKUP_DIR`] are never
/// linted. A document the engine cannot read, or one it could not check in full, fails
/// the lint.
pub struct EngineLinter {
    engine: Engine,
    renames: RenameTable,
}

impl EngineLinter {
    /// An engine over `sets`.
    ///
    /// # Errors
    /// [`Error::Lint`] when the engine refuses the rule sets.
    pub fn new(sets: Vec<RuleSet>) -> Result<Self, Error> {
        // STUB(slopvac-8l4.16.12): both sides run under the core defaults. Each side
        // should apply its own config once the config to engine settings bridge exists.
        let settings = Settings::new(
            NonEmptyString::new("normal").map_err(|e| Error::Lint(e.to_string()))?,
            GenreResult {
                id: GenreId::new("reference").map_err(|e| Error::Lint(e.to_string()))?,
                source: GenreSource::Default,
                reasons: Vec::new(),
            },
        );
        let engine = Engine::new(sets, settings).map_err(|e| Error::Lint(e.to_string()))?;
        Ok(Self {
            engine,
            renames: RenameTable::bundled(),
        })
    }
}

impl std::fmt::Debug for EngineLinter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineLinter").finish_non_exhaustive()
    }
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// Collect the Markdown and MDX files under `dir`, files before sub-directories, each
/// sorted by name. Like the planner, the walk never enters [`BACKUP_DIR`], at any depth,
/// or a directory the engine's discovery skips. Symbolic links are not followed.
fn walk(dir: &Path, out: &mut Vec<(PathBuf, FileKind)>) -> Result<(), Error> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| Error::io(dir, e))?
        .collect::<Result<_, _>>()
        .map_err(|e| Error::io(dir, e))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    let mut dirs = Vec::new();
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = entry.file_type().map_err(|e| Error::io(&path, e))?;
        if kind.is_dir() {
            if name != BACKUP_DIR && !discover::SKIP_DIRS.contains(&name.as_ref()) {
                dirs.push(path);
            }
        } else if kind.is_file()
            && let Some(kind @ (FileKind::Markdown | FileKind::Mdx)) =
                FileKind::from_file_name(&name)
        {
            out.push((path, kind));
        }
    }
    for dir in dirs {
        walk(&dir, out)?;
    }
    Ok(())
}

/// The first warning-level notice, as a failure: part of the document was not checked.
fn unchecked(label: &str, notices: &[Notice]) -> Result<(), Error> {
    match notices.iter().find(|n| n.level == NoticeLevel::Warning) {
        Some(n) => Err(Error::Lint(format!(
            "{label}: not fully checked: {}: {}",
            n.code.as_str(),
            n.message.as_str()
        ))),
        None => Ok(()),
    }
}

impl Linter for EngineLinter {
    fn lint(
        &self,
        root: &Path,
        phase: Phase,
        overlay: &[(PathBuf, String)],
    ) -> Result<Vec<Finding>, Error> {
        let base = canonical(root);
        let mut found = Vec::new();
        walk(root, &mut found)?;
        let rewritten: HashMap<PathBuf, &str> = match phase {
            Phase::Before => HashMap::new(),
            Phase::After => overlay
                .iter()
                .map(|(path, text)| (canonical(path), text.as_str()))
                .collect(),
        };
        let mut docs = Vec::new();
        for (file, kind) in found {
            let path = canonical(&file);
            let label = path.strip_prefix(&base).unwrap_or(&path).to_owned();
            let text = match (rewritten.get(&path), phase) {
                (Some(text), _) => (*text).to_owned(),
                (None, phase) => {
                    let text =
                        fs::read_to_string(&path).map_err(|e| Error::io(&root.join(&label), e))?;
                    match phase {
                        Phase::Before => rewrite_directives(&text, &self.renames).0,
                        Phase::After => text,
                    }
                }
            };
            docs.push((label.display().to_string(), kind, text));
        }
        let inputs: Vec<Input<'_>> = docs
            .iter()
            .map(|(label, kind, text)| match kind {
                FileKind::Mdx => Input::mdx(label, text),
                _ => Input::markdown(label, text),
            })
            .collect();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for (result, (label, _, _)) in self.engine.lint_all(&inputs).into_iter().zip(&docs) {
            let doc = result.map_err(|e| Error::Lint(format!("{label}: {e}")))?;
            unchecked(label, &doc.unchecked)?;
            for finding in doc.findings {
                *counts.entry(finding.rule_id.to_string()).or_default() += 1;
            }
        }
        Ok(counts
            .into_iter()
            .map(|(rule, count)| Finding { rule, count })
            .collect())
    }
}
