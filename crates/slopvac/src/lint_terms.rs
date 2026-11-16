//! The term inputs of an engine: project terms files, blocklists and the term-casing rule
//! pack, from the resolved `terms` settings.
//!
//! `terms.allow` already reaches the engine as `Settings::allowed_words`. `terms.files` and
//! `terms.blocklists` are paths relative to the file that holds the value
//! (docs/contract/config.md), so each is resolved against that file's directory. Every engine
//! the CLI builds, for `lint` and for the language server, goes through [`build_engine`].

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value as Json;
use slopvac_config::Resolved;
use slopvac_engine::{Engine, Settings, TERMS_PACK, terms_rule_set};
use slopvac_model::PackId;
use slopvac_rules::RuleSet;
use slopvac_terms::{Blocklist, TermsFile};

use crate::errors::CliError;
use crate::redact;

/// The files that feed an engine's term checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TermInputs {
    /// `terms.files`, in priority order.
    pub files: Vec<PathBuf>,
    /// `terms.blocklists`, in priority order.
    pub blocklists: Vec<PathBuf>,
}

/// The term-casing rule pack, for name checks against the catalogue.
pub fn casing_pack() -> &'static RuleSet {
    static PACK: OnceLock<RuleSet> = OnceLock::new();
    PACK.get_or_init(|| {
        terms_rule_set(&PackId::new(TERMS_PACK).expect("the terms pack id is valid"))
            .expect("the shipped terms pack loads")
    })
}

fn paths_at(resolved: &Resolved, key: &str, root: &Path) -> Vec<PathBuf> {
    let path = ["terms", key];
    let Some(list) = resolved.get(&path).and_then(Json::as_array) else {
        return Vec::new();
    };
    // The paths are relative to the file that holds the value: the directory of the config
    // file, or the project root when no file is known. A built-in style names no files.
    let base = match resolved.source(&path).and_then(|s| s.file.as_deref()) {
        Some(file) if file.starts_with("preset:") => return Vec::new(),
        Some(file) => root
            .join(file)
            .parent()
            .map_or_else(|| root.to_path_buf(), Path::to_path_buf),
        None => root.to_path_buf(),
    };
    list.iter()
        .filter_map(Json::as_str)
        .map(|p| base.join(p))
        .collect()
}

/// The term files and blocklists the resolved settings name. `root` is the directory of the
/// project config, which the config sources are relative to.
#[must_use]
pub fn inputs(resolved: &Resolved, root: &Path) -> TermInputs {
    TermInputs {
        files: paths_at(resolved, "files", root),
        blocklists: paths_at(resolved, "blocklists", root),
    }
}

/// Fill the message of a terms error with a path that is safe to show.
fn failed(path: &Path, what: &str, message: &str, cwd: &Path) -> CliError {
    let shown = redact::shown_path(path, cwd);
    CliError::new(
        "terms-invalid",
        format!(
            "{shown}: {what}: {}",
            message.replace(&path.display().to_string(), &shown)
        ),
    )
}

/// The rule sets an engine runs: the core rules and the term-casing pack.
///
/// # Errors
/// `engine-unavailable` when the shipped term pack does not load.
pub fn rule_sets(core: RuleSet) -> Result<Vec<RuleSet>, CliError> {
    let terms = terms_rule_set(&PackId::new(TERMS_PACK).expect("the terms pack id is valid"))
        .map_err(|e| CliError::new("engine-unavailable", e.to_string()))?;
    Ok(vec![core, terms])
}

/// Build the engine for `sets` and `settings` with the project's terms and blocklists.
///
/// # Errors
/// `terms-invalid` for a terms file or blocklist that cannot be read or is malformed, and
/// `engine-unavailable` when the engine cannot be built.
pub fn build_engine(
    sets: Vec<RuleSet>,
    settings: Settings,
    terms: &TermInputs,
    cwd: &Path,
) -> Result<Engine, CliError> {
    for file in &terms.files {
        TermsFile::load(file)
            .map_err(|e| failed(file, "cannot load the terms file", &e.to_string(), cwd))?;
    }
    let mut blocklist = Blocklist::default();
    for file in &terms.blocklists {
        let loaded = Blocklist::load(file)
            .map_err(|e| failed(file, "cannot load the blocklist", &e.to_string(), cwd))?;
        blocklist.merge(loaded);
    }
    let unavailable =
        |e: slopvac_engine::EngineError| CliError::new("engine-unavailable", e.to_string());
    let mut engine = Engine::new(sets, settings).map_err(unavailable)?;
    if !terms.files.is_empty() {
        engine = engine.with_term_files(&terms.files).map_err(unavailable)?;
    }
    if !blocklist.is_empty() {
        engine = engine.with_blocklist(&blocklist).map_err(unavailable)?;
    }
    Ok(engine)
}
