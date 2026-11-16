//! The [`Linter`] over `slopvac-engine`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use slopvac_engine::discover::{self, FileKind, Selection};
use slopvac_engine::{Engine, Input, Settings};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString};
use slopvac_rules::RuleSet;

use crate::{BACKUP_DIR, Error, Finding, Linter, Phase};

/// Lints the Markdown and MDX documents of a project with the engine.
///
/// `Before` reads every document from disk. `After` reads a document from the overlay
/// when the plan rewrites it. Files under [`BACKUP_DIR`] are never linted.
pub struct EngineLinter {
    engine: Engine,
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
        Ok(Self { engine })
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

impl Linter for EngineLinter {
    fn lint(
        &self,
        root: &Path,
        phase: Phase,
        overlay: &[(PathBuf, String)],
    ) -> Result<Vec<Finding>, Error> {
        let base = canonical(root);
        let shown = root.display().to_string();
        let found = discover::discover(
            &[shown.as_str()],
            discover::Options {
                selection: Selection::Prose,
                ..discover::Options::default()
            },
        )
        .map_err(|e| Error::Lint(e.to_string().replace(&shown, ".")))?;
        let rewritten: HashMap<PathBuf, &str> = match phase {
            Phase::Before => HashMap::new(),
            Phase::After => overlay
                .iter()
                .map(|(path, text)| (canonical(path), text.as_str()))
                .collect(),
        };
        let backup = base.join(BACKUP_DIR);
        let mut docs = Vec::new();
        for file in found.files {
            if !matches!(file.kind, FileKind::Markdown | FileKind::Mdx) {
                continue;
            }
            let path = canonical(&file.path);
            if path.starts_with(&backup) {
                continue;
            }
            let label = path.strip_prefix(&base).unwrap_or(&path).to_owned();
            let text = match rewritten.get(&path) {
                Some(text) => (*text).to_owned(),
                None => fs::read_to_string(&path).map_err(|e| Error::io(&root.join(&label), e))?,
            };
            docs.push((label.display().to_string(), file.kind, text));
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
