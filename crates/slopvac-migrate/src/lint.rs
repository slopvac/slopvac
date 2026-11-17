//! The [`Linter`] over `slopvac-engine`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use slopvac_engine::discover::FileKind;
use slopvac_engine::{Engine, Input, Settings};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString, Notice, NoticeLevel};
use slopvac_rules::RuleSet;

use crate::capped::Capped;
use crate::limits::{Budget, Kind, Limits};
use crate::walk::discover;
use crate::{Error, Finding, Linter, Phase};

/// Lints the Markdown and MDX documents of a project with the engine.
///
/// `Before` reads every document from disk. `After` reads a document from the overlay
/// when the plan rewrites it. Files under [`BACKUP_DIR`] are never linted. A document
/// the engine cannot read, or one it could not check in full, fails the lint.
pub struct EngineLinter {
    engine: Engine,
    limits: Limits,
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
            limits: Limits::default(),
        })
    }
}

impl EngineLinter {
    /// The same linter under other per-run limits.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
}

impl std::fmt::Debug for EngineLinter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineLinter").finish_non_exhaustive()
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
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
        let mut budget = Budget::new(self.limits);
        // The planner reports what discovery excludes; the lint only needs the documents.
        let found = discover(root, &mut budget, &mut Capped::new("warnings"))?.docs;
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
            let shown = root.join(&label);
            let text = match rewritten.get(&path) {
                Some(text) => {
                    budget.check_text(&shown, Kind::Document, text)?;
                    (*text).to_owned()
                }
                None => budget
                    .read(&path, Kind::Document)
                    .map_err(|e| e.at(&shown))?,
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
