//! The term inputs of an engine: project terms files, blocklists and the term-casing rule
//! pack, from the resolved `terms` settings.
//!
//! `terms.allow` already reaches the engine as `Settings::allowed_words`. `terms.files` and
//! `terms.blocklists` are paths relative to the file that holds the value
//! (docs/contract/config.md), so each is resolved against that file's directory. Every engine
//! the CLI builds, for `lint` and for the language server, goes through [`build_engine`].

use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use slopvac_config::Resolved;
use slopvac_engine::{Engine, Settings, TERMS_PACK, terms_rule_set};
use slopvac_model::PackId;
use slopvac_rules::RuleSet;
use slopvac_terms::{Blocklist, TermsError, TermsFile};

use crate::errors::CliError;

/// The files that feed an engine's term checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TermInputs {
    /// `terms.files`, in priority order.
    pub files: Vec<PathBuf>,
    /// `terms.blocklists`, in priority order.
    pub blocklists: Vec<PathBuf>,
}

/// The term-casing rule pack.
///
/// # Errors
/// `engine-unavailable` when the shipped term pack does not load.
pub fn casing_rule_set() -> Result<RuleSet, CliError> {
    terms_rule_set(&PackId::new(TERMS_PACK).expect("the terms pack id is valid"))
        .map_err(|e| CliError::new("engine-unavailable", e.to_string()))
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

/// A terms error as a message that holds no word, term or key from the file, and a path that is
/// safe to show. The file's text never reaches stderr, a report or the language server: only
/// the entry number and the kind of failure do.
fn failed(path: &Path, what: &str, error: &TermsError, cwd: &Path) -> CliError {
    let shown = slopvac_fs::display_under_root(path, cwd);
    let detail = match error {
        TermsError::Io { .. } => "cannot be read".to_owned(),
        TermsError::TooLarge { cap, .. } => format!("is larger than {cap} bytes"),
        TermsError::Parse { .. } => "is malformed".to_owned(),
        TermsError::MissingSchemaVersion { .. } => {
            "has no `schema_version`; set `schema_version = 3`".to_owned()
        }
        TermsError::SchemaVersion { .. } => {
            "has an unsupported `schema_version`; set `schema_version = 3`".to_owned()
        }
        TermsError::UnknownKey { .. } => "has an unknown key".to_owned(),
        TermsError::Entry {
            number: Some(number),
            problem,
            ..
        } => format!("entry {number} {}", problem.describe()),
        TermsError::Entry { problem, .. } => format!("has an entry that {}", problem.describe()),
    };
    // The path is the user's own text, so it is escaped for the terminal. The report keeps it raw.
    let text = |shown: &str| format!("{shown}: {what}: {detail}");
    CliError::new("terms-invalid", text(&slopvac_sanitize::line(&shown)))
        .with_report_message(&text(&shown))
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
    build_engine_reading(sets, settings, terms, cwd, &TermsFile::load)
}

/// [`build_engine`] with the terms file read passed in, so a test can change a file between
/// the one read and anything that would read it again.
fn build_engine_reading(
    sets: Vec<RuleSet>,
    settings: Settings,
    terms: &TermInputs,
    cwd: &Path,
    read_terms: &dyn Fn(&Path) -> Result<TermsFile, TermsError>,
) -> Result<Engine, CliError> {
    // Each file is read once, and the engine gets the entries that passed validation. A second
    // read by the engine could fail with a message that holds the file's text.
    let mut entries = Vec::new();
    for file in &terms.files {
        let loaded =
            read_terms(file).map_err(|e| failed(file, "cannot load the terms file", &e, cwd))?;
        entries.extend(loaded.terms);
    }
    let mut blocklist = Blocklist::default();
    for file in &terms.blocklists {
        let loaded = Blocklist::load(file)
            .map_err(|e| failed(file, "cannot load the blocklist", &e, cwd))?;
        blocklist.merge(loaded);
    }
    let unavailable =
        |e: slopvac_engine::EngineError| CliError::new("engine-unavailable", e.to_string());
    let mut engine = Engine::new(sets, settings).map_err(unavailable)?;
    if !terms.files.is_empty() {
        engine = engine.with_term_entries(entries);
    }
    if !blocklist.is_empty() {
        // The engine's message can quote a word of the blocklist, so it is not forwarded.
        engine = engine.with_blocklist(&blocklist).map_err(|_| {
            CliError::new("engine-unavailable", "a blocklist entry cannot be compiled")
        })?;
    }
    Ok(engine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint_settings::{self, Overrides};
    use crate::rule_catalog::Catalog;
    use slopvac_config::LoadedConfig;
    use slopvac_engine::Input;

    const SENTINEL: &str = "SENTINEL-NOT-A-TERMS-FILE {{{";

    #[test]
    fn a_validated_terms_file_is_not_read_again() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("terms.toml");
        std::fs::write(
            &file,
            "schema_version = 3\n\n[[terms]]\nterm = \"Zorblax\"\n",
        )
        .expect("terms");
        let config = LoadedConfig::without_files(dir.path().to_path_buf());
        let resolved = lint_settings::resolve(
            &config,
            Path::new("doc.md"),
            &lint_settings::DocumentLayers::default(),
        );
        let catalog = Catalog::load().expect("the rules load");
        let settings = lint_settings::build(&catalog, &resolved, Vec::new(), &Overrides::default())
            .expect("settings");
        let inputs = TermInputs {
            files: vec![file.clone()],
            blocklists: Vec::new(),
        };
        // The one read returns the valid file, then the file on disk becomes unreadable
        // content. Anything that read the path again would fail on the sentinel.
        let read_once = |path: &Path| {
            let loaded = TermsFile::load(path);
            std::fs::write(path, SENTINEL).expect("replace the file");
            loaded
        };
        let engine = build_engine_reading(
            catalog.into_sets(),
            settings,
            &inputs,
            dir.path(),
            &read_once,
        )
        .unwrap_or_else(|e| panic!("the validated entries govern the engine: {e}"));
        let doc = engine
            .lint(&Input::markdown(
                "doc.md",
                "The team deploys zorblax today.\n",
            ))
            .expect("lint");
        let matched: Vec<&str> = doc
            .findings
            .iter()
            .filter(|f| f.rule_id.as_str() == "terms:terms.casing")
            .map(|f| f.matched_text.as_str())
            .collect();
        assert_eq!(matched, ["zorblax"]);
    }
}
