//! The lint backend: discovery, configuration, the engine and report assembly.
//!
//! The CLI codes against [`Linter`]. [`EngineLinter`] discovers the files, loads the nearest
//! `slopvac.toml` for each, builds one [`slopvac_engine::Engine`] per distinct settings, lints
//! every input and assembles the v3 [`Report`] with its summary and gate.
//!
//! Rules come from [`crate::rule_catalog`]; see that module for what is still a stub.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use slopvac_config::{ConfigError, Discovery, Inputs, LoadedConfig, NoPresets};
use slopvac_engine::discover::{DiscoverError, FileKind, Options, Selection, discover};
use slopvac_engine::{Engine, Input, Kind};
use slopvac_model::{
    Document, KebabName, NonEmptyString, Notice, NoticeLevel, Report, SchemaVersion,
};

use crate::errors::CliError;
use crate::lint_settings::{self, Overrides};
use crate::redact;
use crate::rule_catalog;
use crate::stdin::StdinPlan;
use crate::summary::summarize;

/// What the CLI asks the engine to lint.
#[derive(Debug, Clone, Default)]
pub struct LintRequest {
    /// Project root. Paths outside it are redacted in messages.
    pub root: PathBuf,
    /// Files to lint. Empty with `stdin` set means standard input.
    pub targets: Vec<PathBuf>,
    /// Raw stdin bytes, when `--stdin` was given.
    pub stdin: Option<Vec<u8>>,
    /// Parser name and mode for the stdin document, set with `stdin`.
    pub stdin_plan: Option<StdinPlan>,
    /// `--config`: use this file instead of the nearest `slopvac.toml`.
    pub config: Option<PathBuf>,
    /// `--mode code-comments`: lint the comments of source files.
    pub code_comments: bool,
    /// Settings from the command line.
    pub overrides: Overrides,
}

/// The result of one run.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    /// Documents checked.
    pub documents: usize,
    /// Whether at least one gate failed.
    pub gate_failed: bool,
    /// Documents that could not be checked, or checked only in part.
    pub incomplete: Vec<CliError>,
    /// The v3 report for the documents that were checked.
    pub report: Report,
    /// Text of each checked document by report path, for formats that need columns.
    pub sources: HashMap<String, String>,
}

impl Default for RunOutcome {
    fn default() -> Self {
        Self {
            documents: 0,
            gate_failed: false,
            incomplete: Vec::new(),
            report: assemble(&[], Vec::new()),
            sources: HashMap::new(),
        }
    }
}

/// The report of `documents`, with run-level `notices`.
fn assemble(documents: &[Document], notices: Vec<Notice>) -> Report {
    Report {
        schema_version: SchemaVersion,
        slopvac_version: NonEmptyString::new(env!("CARGO_PKG_VERSION"))
            .expect("the package version is not empty"),
        summary: summarize(documents),
        documents: documents.to_vec(),
        notices,
    }
}

/// The engine contract the CLI depends on.
pub trait Linter {
    /// Lint the request.
    fn lint(&self, request: &LintRequest) -> RunOutcome;
}

/// The linter backed by `slopvac-engine`.
#[derive(Debug, Clone, Copy, Default)]
pub struct EngineLinter;

/// One input that is ready to lint.
struct Item {
    shown: String,
    text: String,
    kind: Kind,
    /// Config of the file and the file's path relative to the config root.
    config: Option<(Arc<LoadedConfig>, PathBuf)>,
}

/// The run so far.
#[derive(Default)]
struct Run {
    incomplete: Vec<CliError>,
    notices: Vec<Notice>,
}

impl Run {
    /// Record an input that was not checked: an exit-2 error and a report notice.
    fn skip(&mut self, code: &'static str, shown: &str, message: String) {
        let error = CliError::new(code, message);
        self.notices.push(Notice {
            code: KebabName::new(code).expect("skip codes are kebab-case"),
            level: NoticeLevel::Warning,
            message: NonEmptyString::new(error.message.clone()).expect("messages are not empty"),
            path: Some(shown.to_owned()),
        });
        self.incomplete.push(error);
    }
}

/// The engine kind for a discovered file, or why there is none.
fn kind_of_file(kind: &FileKind) -> Result<Kind, &'static str> {
    match kind {
        FileKind::Markdown => Ok(Kind::Markdown),
        FileKind::Mdx => Ok(Kind::Mdx),
        FileKind::Rst => Ok(Kind::Rst),
        // STUB(slopvac-8l4.12.28): HTML and plain text have no engine parser yet, and
        // comment-mode rules run once slopvac-8l4.3.26 lands.
        FileKind::Html | FileKind::Text | FileKind::Toml | FileKind::Source(_) => {
            Err("this file type has no engine parser yet")
        }
    }
}

fn kind_of_language(plan: &StdinPlan) -> Result<Kind, &'static str> {
    if plan.mode != "prose" {
        return Err("comment mode has no engine parser yet");
    }
    match plan.language.as_str() {
        "markdown" => Ok(Kind::Markdown),
        "mdx" => Ok(Kind::Mdx),
        "rst" => Ok(Kind::Rst),
        _ => Err("this language has no engine parser yet"),
    }
}

fn config_error(err: &ConfigError) -> CliError {
    let mut message = err.to_string();
    if err.is_legacy() {
        message.push_str(" Run `slopvac migrate` to convert it.");
    }
    CliError::new(err.code().as_str(), message)
}

/// Finds the config of each directory once.
struct Configs<'a> {
    discovery: Discovery<'a>,
    explicit: Option<Arc<LoadedConfig>>,
}

impl Configs<'_> {
    fn of(&self, dir: &Path) -> Result<Option<Arc<LoadedConfig>>, ConfigError> {
        match &self.explicit {
            Some(config) => Ok(Some(Arc::clone(config))),
            None => self.discovery.nearest(dir),
        }
    }
}

impl Linter for EngineLinter {
    fn lint(&self, request: &LintRequest) -> RunOutcome {
        let mut run = Run::default();
        let presets = NoPresets;
        let mut configs = Configs {
            discovery: Discovery::new(&presets),
            explicit: None,
        };
        if let Some(given) = &request.config {
            match configs.discovery.explicit_in(&request.root, given) {
                Ok(config) => configs.explicit = Some(config),
                Err(err) => {
                    // A config that cannot be read lints nothing (policies V7).
                    return failed(request, run, &config_error(&err));
                }
            }
        }
        let items = match collect(request, &configs, &mut run) {
            Ok(items) => items,
            Err(err) => return failed(request, run, &err),
        };
        lint_items(request, items, run)
    }
}

/// An outcome for a run that could not start.
fn failed(_request: &LintRequest, mut run: Run, error: &CliError) -> RunOutcome {
    run.incomplete.push(error.clone());
    RunOutcome {
        incomplete: run.incomplete,
        report: assemble(&[], run.notices),
        ..RunOutcome::default()
    }
}

fn collect(
    request: &LintRequest,
    configs: &Configs<'_>,
    run: &mut Run,
) -> Result<Vec<Item>, CliError> {
    let mut items = Vec::new();
    if let Some(bytes) = &request.stdin {
        collect_stdin(request, bytes, configs, run, &mut items)?;
    }
    collect_targets(request, configs, run, &mut items)?;
    Ok(items)
}

fn collect_stdin(
    request: &LintRequest,
    bytes: &[u8],
    configs: &Configs<'_>,
    run: &mut Run,
    items: &mut Vec<Item>,
) -> Result<(), CliError> {
    let shown = redact::STDIN_NAME;
    let kind = match &request.stdin_plan {
        Some(plan) => kind_of_language(plan),
        None => Ok(Kind::Markdown),
    };
    match (kind, String::from_utf8(bytes.to_vec())) {
        (Ok(kind), Ok(text)) => items.push(Item {
            shown: shown.to_owned(),
            text,
            kind,
            config: config_for(configs, &request.root, Path::new(shown))?,
        }),
        (Err(why), _) => run.skip("input-unsupported", shown, format!("{shown}: {why}")),
        (_, Err(_)) => run.skip("input-not-utf8", shown, format!("{shown}: not valid UTF-8")),
    }
    Ok(())
}

fn collect_targets(
    request: &LintRequest,
    configs: &Configs<'_>,
    run: &mut Run,
    items: &mut Vec<Item>,
) -> Result<(), CliError> {
    let selection = if request.code_comments {
        Selection::CodeComments
    } else {
        Selection::Prose
    };
    let mut seen = HashSet::new();
    for target in &request.targets {
        let shown_target = redact::shown_path(target, &request.root);
        let found = match discover(
            &[target.to_string_lossy().into_owned()],
            Options {
                selection,
                ..Options::default()
            },
        ) {
            Ok(found) => found,
            Err(DiscoverError::NoSuchTarget(_)) => {
                run.skip(
                    "input-unreadable",
                    &shown_target,
                    format!("{shown_target}: not found"),
                );
                continue;
            }
            Err(err) => {
                run.skip(
                    "input-unreadable",
                    &shown_target,
                    format!("{shown_target}: {err}"),
                );
                continue;
            }
        };
        for skipped in found.skipped {
            let shown = redact::shown_path(&skipped.path, &request.root);
            run.skip(
                "input-unsupported",
                &shown,
                format!("{shown}: {}", skipped.reason),
            );
        }
        for file in found.files {
            let canonical = std::fs::canonicalize(&file.path).unwrap_or_else(|_| file.path.clone());
            if !seen.insert(canonical.clone()) {
                continue;
            }
            let shown = redact::shown_path(&file.path, &request.root);
            let kind = match kind_of_file(&file.kind) {
                Ok(kind) => kind,
                Err(why) => {
                    run.skip("input-unsupported", &shown, format!("{shown}: {why}"));
                    continue;
                }
            };
            let bytes = match std::fs::read(&file.path) {
                Ok(bytes) => bytes,
                Err(err) => {
                    run.skip(
                        "input-unreadable",
                        &shown,
                        format!("{shown}: {}", err.kind()),
                    );
                    continue;
                }
            };
            let Ok(text) = String::from_utf8(bytes) else {
                run.skip(
                    "input-not-utf8",
                    &shown,
                    format!("{shown}: not valid UTF-8"),
                );
                continue;
            };
            let config = config_for(configs, &request.root, &canonical)?;
            if let Some((loaded, rel)) = &config
                && loaded.is_excluded(rel)
                && request.config.is_none()
            {
                continue;
            }
            items.push(Item {
                shown,
                text,
                kind,
                config,
            });
        }
    }
    Ok(())
}

/// The config that applies to `file` and its path relative to the config root.
fn config_for(
    configs: &Configs<'_>,
    root: &Path,
    file: &Path,
) -> Result<Option<(Arc<LoadedConfig>, PathBuf)>, CliError> {
    let absolute = if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    };
    let dir = absolute.parent().unwrap_or(root);
    let dir = if dir.as_os_str().is_empty() {
        root
    } else {
        dir
    };
    let found = configs.of(dir).map_err(|e| config_error(&e))?;
    Ok(found.map(|config| {
        let rel = absolute
            .strip_prefix(&config.root)
            .map_or_else(|_| file.to_path_buf(), Path::to_path_buf);
        (config, rel)
    }))
}

/// Settings and the indexes of the items that share them.
type Groups = BTreeMap<String, (slopvac_engine::Settings, Vec<usize>)>;

/// Group the items by their settings: one engine per distinct settings.
fn group_by_settings(
    request: &LintRequest,
    items: &[Item],
    rules: &slopvac_rules::RuleSet,
) -> Result<Groups, CliError> {
    let mut groups = Groups::new();
    for (index, item) in items.iter().enumerate() {
        let resolved = item.config.as_ref().map(|(config, rel)| {
            config.resolve(
                rel,
                &Inputs {
                    presets: &NoPresets,
                    front_matter: &[],
                    directives: &[],
                    cli: &[],
                    env: &[],
                },
            )
        });
        let files = item
            .config
            .as_ref()
            .map(|(config, _)| config_files(config))
            .unwrap_or_default();
        let settings = lint_settings::build(
            rules,
            resolved.as_ref(),
            lint_settings::config_sources(&files),
            &request.overrides,
        )?;
        groups
            .entry(format!("{settings:?}"))
            .or_insert_with(|| (settings, Vec::new()))
            .1
            .push(index);
    }
    Ok(groups)
}

/// A config that lists `styles` asks for presets that no pack supplies yet
/// (STUB(slopvac-8l4.12.29)). Ignoring them would report a pass the config did not ask for,
/// so the run is incomplete.
fn note_unapplied_styles(items: &[Item], run: &mut Run) {
    let mut seen = HashSet::new();
    for (config, _) in items.iter().filter_map(|i| i.config.as_ref()) {
        let styles = config
            .project_value(&["styles"])
            .and_then(|v| v.as_array().map(Vec::len))
            .unwrap_or(0);
        if styles > 0 && seen.insert(config.root.clone()) {
            run.skip(
                "styles-not-applied",
                "slopvac.toml",
                format!(
                    "slopvac.toml: `styles` lists {styles} preset(s), but no style pack is \
                     available yet, so none apply"
                ),
            );
        }
    }
}

fn lint_items(request: &LintRequest, items: Vec<Item>, mut run: Run) -> RunOutcome {
    let rules = match rule_catalog::core() {
        Ok(rules) => rules,
        Err(err) => return failed(request, run, &err),
    };
    note_unapplied_styles(&items, &mut run);
    let groups = match group_by_settings(request, &items, &rules) {
        Ok(groups) => groups,
        Err(err) => return failed(request, run, &err),
    };

    let mut documents: Vec<Option<Document>> = vec![None; items.len()];
    let mut spare = Some(rules);
    for (settings, indexes) in groups.into_values() {
        // The engine owns its rule set, and a set does not clone: reload it for later groups.
        let set = match spare.take().map_or_else(rule_catalog::core, Ok) {
            Ok(set) => set,
            Err(err) => return failed(request, run, &err),
        };
        let engine = match Engine::new(vec![set], settings) {
            Ok(engine) => engine.with_project_roots(vec![request.root.clone()]),
            Err(err) => {
                return failed(
                    request,
                    run,
                    &CliError::new("engine-unavailable", err.to_string()),
                );
            }
        };
        let inputs: Vec<Input<'_>> = indexes
            .iter()
            .map(|&i| Input {
                path: &items[i].shown,
                source: &items[i].text,
                kind: items[i].kind,
                bom: slopvac_model::BomMode::Disk,
            })
            .collect();
        for (&index, result) in indexes.iter().zip(engine.lint_all(&inputs)) {
            match result {
                Ok(document) => documents[index] = Some(document),
                Err(err) => {
                    let shown = items[index].shown.clone();
                    run.skip("lint-failed", &shown, format!("{shown}: {err}"));
                }
            }
        }
    }

    let documents: Vec<Document> = documents.into_iter().flatten().collect();
    for document in &documents {
        for notice in &document.unchecked {
            if notice.level == NoticeLevel::Warning {
                // Part of the check did not run, so the result cannot establish a pass.
                run.incomplete.push(CliError::new(
                    "check-incomplete",
                    format!(
                        "{}: {}: {}",
                        document.path,
                        notice.code.as_str(),
                        notice.message
                    ),
                ));
            }
        }
    }
    let sources = items
        .into_iter()
        .map(|item| (item.shown, item.text))
        .collect();
    RunOutcome {
        documents: documents.len(),
        gate_failed: documents.iter().any(|d| !d.gate.is_passed()),
        incomplete: run.incomplete,
        report: assemble(&documents, run.notices),
        sources,
    }
}

/// The rules and the settings of the project config found from `root`, for a long-running
/// consumer such as the language server that lints one buffer at a time.
///
/// # Errors
/// The config error, or `rules-unavailable`.
pub fn project_settings(
    root: &Path,
) -> Result<(slopvac_rules::RuleSet, slopvac_engine::Settings), CliError> {
    let rules = rule_catalog::core()?;
    let presets = NoPresets;
    let discovery = Discovery::new(&presets);
    let found = discovery.nearest(root).map_err(|e| config_error(&e))?;
    let (resolved, files) = match &found {
        Some(config) => (
            Some(config.resolve(
                Path::new(""),
                &Inputs {
                    presets: &presets,
                    front_matter: &[],
                    directives: &[],
                    cli: &[],
                    env: &[],
                },
            )),
            config_files(config),
        ),
        None => (None, Vec::new()),
    };
    let settings = lint_settings::build(
        &rules,
        resolved.as_ref(),
        lint_settings::config_sources(&files),
        &Overrides::default(),
    )?;
    Ok((rules, settings))
}

/// The display name of each config file and whether it was pulled in by `extends`.
fn config_files(config: &LoadedConfig) -> Vec<(String, bool)> {
    config
        .files
        .iter()
        .map(|f| (f.display.clone(), f.layer == slopvac_config::Layer::Extends))
        .collect()
}
