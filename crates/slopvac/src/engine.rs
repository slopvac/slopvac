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

use slopvac_config::{ConfigError, Discovery, LoadedConfig};
use slopvac_engine::discover::{DiscoverError, FileKind, Options, Selection, discover};
use slopvac_engine::{Input, Kind};
use slopvac_model::{
    Document, KebabName, NonEmptyString, Notice, NoticeLevel, Report, SchemaVersion,
};
use slopvac_styles::{BuiltinSource, StylePresets};

use crate::errors::CliError;
use crate::lint_settings::{self, Overrides};
use crate::lint_terms::{self, TermInputs};
use crate::redact;
use crate::rule_catalog::Catalog;
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
    /// The directory that holds the file, canonical. Not set for standard input.
    dir: Option<PathBuf>,
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
            data: serde_json::Map::new(),
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

/// Redact every backtick-quoted absolute path in `text`.
///
/// The config diagnostics quote the user's own value in backticks, such as the value of a bad
/// `locale` or an unknown key. When the quoted text is an absolute path, the whole quoted text
/// is replaced with `redact::shown_path` (policies.md, 1.7).
fn redact_quoted(text: &str, root: &Path) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let (head, tail) = rest.split_at(open);
        out.push_str(head);
        let tail = &tail[1..];
        let Some(close) = tail.find('`') else {
            out.push('`');
            rest = tail;
            continue;
        };
        let quoted = &tail[..close];
        out.push('`');
        if Path::new(quoted).is_absolute() {
            out.push_str(&redact::shown_path(Path::new(quoted), root));
        } else {
            out.push_str(quoted);
        }
        out.push('`');
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

/// A refused config from its structured fields: each diagnostic's message, its
/// `file:line:column` pointer and its help. The source excerpt is left out because it quotes
/// the user's own text, paths included.
fn describe_rejected(rejected: &slopvac_config::Rejected, root: &Path) -> String {
    use std::fmt::Write as _;
    rejected
        .diagnostics
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let (line, column) = d.location(&rejected.text);
            let mut text = if i == 0 {
                String::new()
            } else {
                format!("error[{}]: ", d.code.as_str())
            };
            text.push_str(&redact_quoted(&d.message, root));
            let _ = write!(text, "\n --> {}:{line}:{column}", rejected.file);
            if let Some(help) = &d.help {
                text.push_str("\nhelp: ");
                text.push_str(&redact_quoted(help, root));
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn config_error(err: &ConfigError, root: &Path) -> CliError {
    let mut message = match err {
        ConfigError::Rejected(rejected) => describe_rejected(rejected, root),
        ConfigError::Io { path, .. } | ConfigError::TooLarge { path, .. } => {
            // The path is the label the caller gave, which may be absolute.
            err.to_string()
                .replace(path.as_str(), &redact::shown_path(Path::new(path), root))
        }
    };
    if err.is_legacy() {
        message.push_str("\nhelp: run `slopvac migrate` to convert it");
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
        let presets = StylePresets(BuiltinSource);
        let mut configs = Configs {
            discovery: Discovery::new(&presets),
            explicit: None,
        };
        if let Some(given) = &request.config {
            let shown = redact::shown_path(given, &request.root);
            if !request.root.join(given).is_file() {
                let error = CliError::new(
                    "config-not-found",
                    format!("--config {shown}: no such file"),
                );
                return failed(request, run, &error);
            }
            match configs.discovery.explicit_in(&request.root, given) {
                Ok(config) => configs.explicit = Some(config),
                Err(err) => {
                    // A config that cannot be read lints nothing (policies V7).
                    return failed(request, run, &config_error(&err, &request.root));
                }
            }
        }
        let rules = match Catalog::load() {
            Ok(rules) => rules,
            Err(err) => return failed(request, run, &err),
        };
        if let Err(err) = validate_configs(request, &configs, &rules) {
            return failed(request, run, &err);
        }
        let items = match collect(request, &configs, &mut run) {
            Ok(items) => items,
            Err(err) => return failed(request, run, &err),
        };
        lint_items(request, items, rules, run)
    }
}

/// Check every config the run reaches against the catalogue, even when no input is left to
/// lint: the config of each target, of standard input and the one named with `--config`.
fn validate_configs(
    request: &LintRequest,
    configs: &Configs<'_>,
    rules: &Catalog,
) -> Result<(), CliError> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if request.stdin.is_some() {
        dirs.push(request.root.clone());
    }
    for target in &request.targets {
        let path = request.root.join(target);
        // Every file a target reaches, whether it names a file, a directory or a glob, has
        // its own nearest config. All of them are checked, before any exclusion drops a file.
        if let Ok(found) = discover(
            &[target.to_string_lossy().into_owned()],
            Options::default().with_selection(if request.code_comments {
                Selection::CodeComments
            } else {
                Selection::Prose
            }),
        ) {
            dirs.extend(found.files.iter().filter_map(|f| {
                let parent = f.path.parent()?;
                Some(request.root.join(parent))
            }));
        }
        if path.is_dir() {
            dirs.push(path);
        } else if let Some(parent) = path.parent().filter(|p| p.is_dir()) {
            dirs.push(parent.to_path_buf());
        }
    }
    dirs.sort();
    dirs.dedup();
    if dirs.is_empty() {
        dirs.push(request.root.clone());
    }
    let mut seen = HashSet::new();
    for dir in dirs {
        let found = configs
            .of(&dir)
            .map_err(|e| config_error(&e, &request.root))?;
        if let Some(config) = found
            && seen.insert(config.root.clone())
        {
            lint_settings::validate(&config, rules)?;
        }
    }
    Ok(())
}

/// An outcome for a run that could not start.
fn failed(_request: &LintRequest, mut run: Run, error: &CliError) -> RunOutcome {
    // The report must not read as a clean run: a notice marks it unsuccessful.
    run.notices.push(Notice {
        code: KebabName::new(error.code)
            .unwrap_or_else(|_| KebabName::new("lint-failed").expect("lint-failed is kebab-case")),
        level: NoticeLevel::Warning,
        message: NonEmptyString::new(error.message.clone())
            .unwrap_or_else(|_| NonEmptyString::new("lint failed").expect("not empty")),
        path: None,
        data: serde_json::Map::new(),
    });
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
    unique_identities(&mut items);
    Ok(items)
}

/// Two files outside the project can share a redacted name such as `<external>/a.md`. Each
/// keeps a distinct identity in the report, so one does not replace the other in the source
/// map that column-aware formats read. Every name in use is reserved first, so a number added
/// to one file's name never lands on another file's own name.
fn unique_identities(items: &mut [Item]) {
    let mut taken: HashSet<String> = items.iter().map(|i| i.shown.clone()).collect();
    let mut first_seen: HashSet<String> = HashSet::new();
    for item in items {
        if first_seen.insert(item.shown.clone()) {
            continue;
        }
        let original = item.shown.clone();
        let mut n = 2;
        let renamed = loop {
            let candidate = match original.strip_prefix("<external>/") {
                Some(rest) => format!("<external>/{n}/{rest}"),
                None => format!("{original}#{n}"),
            };
            if !taken.contains(&candidate) {
                break candidate;
            }
            n += 1;
        };
        taken.insert(renamed.clone());
        item.shown = renamed;
    }
}

/// A discovery failure as one line, with paths in the form every other message uses.
fn discover_message(err: &DiscoverError, root: &Path) -> String {
    let shown = |path: &Path| redact::shown_path(path, root);
    match err {
        DiscoverError::NoSuchTarget(target) => {
            format!("{}: not found", shown(Path::new(target)))
        }
        DiscoverError::UnsupportedSource { path, message } => {
            format!("{}: {message}", shown(path))
        }
        DiscoverError::TooManyAlternatives { pattern, limit } => {
            format!(
                "glob {} expands to more than {limit} alternatives",
                shown(Path::new(pattern))
            )
        }
        DiscoverError::InvalidGlob { pattern, .. } => {
            format!("invalid glob {}", shown(Path::new(pattern)))
        }
        DiscoverError::Walk { path, .. } => {
            format!("{}: cannot walk the directory", shown(path))
        }
    }
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
            dir: None,
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
            Options::default().with_selection(selection),
        ) {
            Ok(found) => found,
            Err(err) => {
                run.skip(
                    "input-unreadable",
                    &shown_target,
                    discover_message(&err, &request.root),
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
            let config = config_for(configs, &request.root, &canonical)?;
            // Exclusions apply first, whether the config was found or named with `--config`,
            // so an excluded file is never read, parsed or reported.
            if let Some((loaded, rel)) = &config
                && loaded.is_excluded(rel)
            {
                continue;
            }
            let kind = match kind_of_file(&file.kind) {
                Ok(kind) => kind,
                Err(why) => {
                    run.skip("input-unsupported", &shown, format!("{shown}: {why}"));
                    continue;
                }
            };
            let read = std::fs::File::open(&file.path)
                .map_err(|e| slopvac_fs::Error::read(&e))
                .and_then(|f| slopvac_fs::read_capped(f, slopvac_fs::MAX_DOCUMENT_BYTES));
            let bytes = match read {
                Ok(bytes) => bytes,
                Err(slopvac_fs::Error::CapExceeded { .. }) => {
                    run.skip(
                        "document-too-large",
                        &shown,
                        format!("{shown}: larger than the 64 MiB document limit"),
                    );
                    continue;
                }
                Err(_) => {
                    run.skip("input-unreadable", &shown, format!("{shown}: cannot read"));
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
            items.push(Item {
                shown,
                text,
                kind,
                config,
                dir: canonical.parent().map(Path::to_path_buf),
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
    let found = configs.of(dir).map_err(|e| config_error(&e, root))?;
    Ok(found.map(|config| {
        let rel = absolute
            .strip_prefix(&config.root)
            .map_or_else(|_| file.to_path_buf(), Path::to_path_buf);
        (config, rel)
    }))
}

/// Settings and the indexes of the items that share them.
type Groups = BTreeMap<String, (slopvac_engine::Settings, TermInputs, Vec<usize>)>;

/// Group the items by their settings: one engine per distinct settings.
fn group_by_settings(
    request: &LintRequest,
    items: &[Item],
    rules: &Catalog,
) -> Result<Groups, CliError> {
    let mut groups = Groups::new();
    let none = LoadedConfig::without_files(request.root.clone());
    for (index, item) in items.iter().enumerate() {
        let (config, rel) = match &item.config {
            Some((config, rel)) => (config.as_ref(), rel.clone()),
            None => (&none, PathBuf::from(&item.shown)),
        };
        let resolved = lint_settings::resolve(config, &rel);
        let files = config_files(config);
        let settings = lint_settings::build(
            rules,
            &resolved,
            lint_settings::config_sources(&files),
            &request.overrides,
        )?;
        let terms = lint_terms::inputs(&resolved, &config.root);
        groups
            .entry(format!("{settings:?}{terms:?}"))
            .or_insert_with(|| (settings, terms, Vec::new()))
            .2
            .push(index);
    }
    Ok(groups)
}

/// The directories an RST interpreter is never taken from: the working directory, the
/// directory of every input and of every named directory, and the repository root of each.
/// A repository outside the working directory is untrusted code, and a `python` it ships on
/// `PATH` must not run.
fn project_roots(request: &LintRequest, items: &[Item]) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = vec![request.root.clone()];
    let mut starts: Vec<PathBuf> = items.iter().filter_map(|i| i.dir.clone()).collect();
    for target in &request.targets {
        let absolute = request.root.join(target);
        if let Ok(dir) = std::fs::canonicalize(&absolute)
            && dir.is_dir()
        {
            starts.push(dir);
        }
    }
    for start in starts {
        if let Some(repo) = start.ancestors().find(|d| d.join(".git").exists()) {
            roots.push(repo.to_path_buf());
        }
        roots.push(start);
    }
    roots.sort();
    roots.dedup();
    roots
}

fn lint_items(request: &LintRequest, items: Vec<Item>, rules: Catalog, mut run: Run) -> RunOutcome {
    let groups = match group_by_settings(request, &items, &rules) {
        Ok(groups) => groups,
        Err(err) => return failed(request, run, &err),
    };

    let mut documents: Vec<Option<Document>> = vec![None; items.len()];
    let mut spare = Some(rules);
    for (settings, terms, indexes) in groups.into_values() {
        // The engine owns its rule sets, and a set does not clone: reload them for later groups.
        let sets = match spare.take().map_or_else(Catalog::load, Ok) {
            Ok(catalog) => catalog.into_sets(),
            Err(err) => return failed(request, run, &err),
        };
        let engine = match lint_terms::build_engine(sets, settings, &terms, &request.root) {
            Ok(engine) => engine.with_project_roots(project_roots(request, &items)),
            Err(err) => return failed(request, run, &err),
        };
        let inputs: Vec<Input<'_>> = indexes
            .iter()
            .map(|&i| Input::new(&items[i].shown, &items[i].text).with_kind(items[i].kind))
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

/// The language server's linter for the project config found from `root`: the shipped
/// rules, the term-casing pack, the settings, the project's terms files and the blocklists,
/// built the same way `slopvac lint` builds an engine.
///
/// # Errors
/// The config error, `terms-invalid`, or `rules-unavailable`.
pub fn project_linter(root: &Path) -> Result<slopvac_lsp::EngineLinter, CliError> {
    let core = Catalog::load()?;
    let presets = StylePresets(BuiltinSource);
    let discovery = Discovery::new(&presets);
    let found = discovery
        .nearest(root)
        .map_err(|e| config_error(&e, root))?;
    let (config, files) = match &found {
        Some(config) => (config.as_ref().clone(), config_files(config)),
        None => (LoadedConfig::without_files(root.to_path_buf()), Vec::new()),
    };
    lint_settings::validate(&config, &core)?;
    let resolved = lint_settings::resolve(&config, Path::new(""));
    let settings = lint_settings::build(
        &core,
        &resolved,
        lint_settings::config_sources(&files),
        &Overrides::default(),
    )?;
    let terms = lint_terms::inputs(&resolved, &config.root);
    let sets = core.into_sets();
    let reasons = slopvac_lsp::exception_names(&sets);
    let engine = lint_terms::build_engine(sets, settings, &terms, root)?;
    Ok(slopvac_lsp::EngineLinter::from_engine(engine, reasons))
}

/// The display name of each config file and whether it was pulled in by `extends`.
fn config_files(config: &LoadedConfig) -> Vec<(String, bool)> {
    config
        .files
        .iter()
        .map(|f| (f.display.clone(), f.layer == slopvac_config::Layer::Extends))
        .collect()
}
