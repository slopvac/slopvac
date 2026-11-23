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

use crate::lint_messages::{LintFixMessage, LintFixReason};
use slopvac_config::{ConfigError, Discovery, LoadedConfig};
use slopvac_engine::discover::{DiscoverError, FileKind, Options, Selection, discover};
use slopvac_engine::{
    AclCheckFailed, Engine, EngineError, FileContext, FileFix, FixError, Input, Kind, Preparsed,
    Root, RootChanged, fix_file_in,
};
use slopvac_messages::Catalog as _;
use slopvac_model::{
    Document, KebabName, NonEmptyString, Notice, NoticeLevel, Report, SchemaVersion,
};
use slopvac_sanitize::line;

use crate::errors::CliError;
use crate::lint_settings::{self, DocumentBlocks, DocumentLayers, Overrides};
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
    /// `--fix`: write the safe replacements back to each file before it is reported.
    pub fix: bool,
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

    /// Explain the settings of each input of the request, without linting it.
    fn explain_config(&self, request: &LintRequest) -> ExplainOutcome;
}

/// The result of `--explain-config`.
#[derive(Debug, Clone, Default)]
pub struct ExplainOutcome {
    /// One block per input, each headed by its redacted path.
    pub text: String,
    /// Inputs that could not be read, or a failure that stopped the run.
    pub incomplete: Vec<CliError>,
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
    /// The file, canonical. Not set for standard input. `--fix` writes here.
    path: Option<PathBuf>,
}

/// The run so far.
#[derive(Default)]
struct Run {
    incomplete: Vec<CliError>,
    notices: Vec<Notice>,
}

impl Run {
    /// Record an input that was not checked: an exit-2 error and a report notice.
    ///
    /// `message` is raw text that may hold a hostile file name. The report formats escape it
    /// for their own output, so the notice keeps it raw. The error goes to stderr as it is,
    /// so it carries the message escaped once.
    fn skip(&mut self, code: &'static str, shown: &str, message: String) {
        let error = CliError::new(code, line(&message));
        self.notices.push(Notice {
            code: KebabName::new(code).expect("skip codes are kebab-case"),
            level: NoticeLevel::Warning,
            message: NonEmptyString::new(message).expect("messages are not empty"),
            path: Some(shown.to_owned()),
            data: serde_json::Map::new(),
        });
        self.incomplete.push(error);
    }

    /// Record what is wrong with a document's `slopvac:` block: an exit-2 error and a report
    /// notice for each diagnostic. The block then contributes nothing and the document is
    /// still linted. The error text is escaped for a terminal; the notice keeps the raw
    /// text, which each report format escapes for itself.
    fn front_matter_errors(&mut self, item: &Item, blocks: &DocumentBlocks) {
        for e in block_errors(&item.shown, &item.text, blocks) {
            self.notices.push(Notice {
                code: KebabName::new(e.code).expect("diagnostic codes are kebab-case"),
                level: NoticeLevel::Warning,
                message: NonEmptyString::new(e.raw).expect("messages are not empty"),
                path: Some(item.shown.clone()),
                data: serde_json::Map::new(),
            });
            self.incomplete.push(CliError::new(e.code, e.escaped));
        }
    }

    /// Record a file the engine could not lint: not checked, so the run exits 2.
    fn skip_engine_error(&mut self, shown: &str, err: &EngineError) {
        // `EngineError` is `#[non_exhaustive]`, so a new variant lands in `lint-failed` until
        // it gets a code of its own.
        let code = match err {
            // The check is incomplete for a reason a pack can fix, so it has a code.
            EngineError::TagLookupBudget { .. } | EngineError::TagBuildBudget { .. } => {
                "context-budget-exceeded"
            }
            _ => "lint-failed",
        };
        self.skip(code, shown, format!("{shown}: {}", err.raw_message()));
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
///
/// `esc` escapes the untrusted parts: the message and help quote the user's own values, and
/// the pointer holds the file's name. The layout newlines and the `error[..]` prefixes are
/// ours and stay.
fn describe_rejected(
    rejected: &slopvac_config::Rejected,
    root: &Path,
    esc: &dyn Fn(&str) -> String,
) -> String {
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
            text.push_str(&esc(&redact_quoted(&d.message, root)));
            let _ = write!(text, "\n --> {}:{line}:{column}", esc(&rejected.file));
            if let Some(help) = &d.help {
                text.push_str("\nhelp: ");
                text.push_str(&esc(&redact_quoted(help, root)));
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text of a refused config, with `esc` applied to every untrusted part.
fn config_message(err: &ConfigError, root: &Path, esc: &dyn Fn(&str) -> String) -> String {
    let mut message = match err {
        ConfigError::Rejected(rejected) => describe_rejected(rejected, root, esc),
        ConfigError::Io { path, .. } | ConfigError::TooLarge { path, .. } => {
            // The path is the label the caller gave, which may be absolute. The rest of the
            // message is fixed wording, so escaping the whole line escapes the path.
            esc(&err
                .to_string()
                .replace(path.as_str(), &redact::shown_path(Path::new(path), root)))
        }
    };
    if err.is_legacy() {
        message.push_str("\nhelp: run `slopvac migrate` to convert it");
    }
    message
}

pub(crate) fn config_error(err: &ConfigError, root: &Path) -> CliError {
    let message = config_message(err, root, &|text| line(text));
    let raw = config_message(err, root, &str::to_owned);
    let error = CliError::new(err.code().as_str(), message).with_report_message(&raw);
    match budget_data(err) {
        Some(data) => error.with_data(data),
        None => error,
    }
}

/// The `budget`, `limit` and `path` keys of a refused config's budget diagnostic.
///
/// Only `ConfigError::Rejected` carries budget data, and the loader builds it with exactly one
/// diagnostic per refusal, so the first diagnostic with data is the only one. The path needs no
/// further redaction: `slopvac-config` already shows it root-relative, or `<external>/<name>`
/// outside the root, and a test pins that for an `extends` target outside the project.
fn budget_data(err: &ConfigError) -> Option<serde_json::Map<String, serde_json::Value>> {
    let ConfigError::Rejected(rejected) = err else {
        return None;
    };
    let budget = rejected
        .diagnostics
        .iter()
        .find_map(|d| d.data.as_deref())?;
    let mut data = serde_json::Map::new();
    data.insert("budget".into(), budget.budget.as_str().into());
    data.insert("limit".into(), budget.limit.into());
    data.insert("path".into(), budget.path.as_str().into());
    Some(data)
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
        // Open the root once, before any file is collected, so a later swap of its path for
        // a link cannot move a fix elsewhere.
        let root = request.fix.then(|| Root::open(&request.root));
        match prepare(request, &mut run) {
            Ok((rules, items)) => lint_items(request, root.as_ref(), items, rules, run),
            Err(err) => failed(request, run, &err),
        }
    }

    fn explain_config(&self, request: &LintRequest) -> ExplainOutcome {
        let mut run = Run::default();
        let items = match prepare(request, &mut run) {
            Ok((_, items)) => items,
            Err(err) => {
                run.incomplete.push(err);
                Vec::new()
            }
        };
        let none = LoadedConfig::without_files(request.root.clone());
        let reads = preparse(request, &items);
        let mut text = String::new();
        for (index, item) in items.iter().enumerate() {
            let (config, rel) = match &item.config {
                Some((config, rel)) => (config.as_ref(), rel.clone()),
                None => (&none, PathBuf::from(&item.shown)),
            };
            // The same document inputs as the lint of the file takes.
            let layers = DocumentLayers::read(
                &item.text,
                &item.shown,
                item.config.as_ref().map(|(c, _)| &**c),
            )
            .with_detected_genre(reads.detected[index].clone());
            let mut explained = lint_settings::resolve(config, &rel, &layers).explain();
            for entry in &mut explained {
                redact_urls(&mut entry.value);
                for replaced in &mut entry.overridden {
                    redact_urls(&mut replaced.value);
                }
            }
            let root = redact::shown_path(&config.root, &request.root);
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&slopvac_sanitize::line(&item.shown));
            text.push('\n');
            text.push_str(&slopvac_config::render_explain(config, &explained, &root));
        }
        ExplainOutcome {
            text,
            incomplete: run.incomplete,
        }
    }
}

/// Strip user-info and query strings from every URL in a config value before it is printed.
fn redact_urls(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => *text = redact::redact_url_value(text),
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_urls),
        serde_json::Value::Object(map) => map.values_mut().for_each(redact_urls),
        _ => {}
    }
}

/// Load the configs, the rules and the inputs of a run, or say why it cannot start.
fn prepare(request: &LintRequest, run: &mut Run) -> Result<(Catalog, Vec<Item>), CliError> {
    let mut configs = Configs {
        discovery: Discovery::new(lint_settings::presets()),
        explicit: None,
    };
    if let Some(given) = &request.config {
        let shown = redact::shown_path(given, &request.root);
        if !request.root.join(given).is_file() {
            return Err(CliError::escaping(
                "config-not-found",
                &format!("--config {shown}: no such file"),
            ));
        }
        // A config that cannot be read lints nothing (policies V7).
        let config = configs
            .discovery
            .explicit_in(&request.root, given)
            .map_err(|err| config_error(&err, &request.root))?;
        configs.explicit = Some(config);
    }
    let rules = Catalog::load()?;
    run.notices
        .extend(validate_configs(request, &configs, &rules)?);
    let items = collect(request, &configs, run)?;
    Ok((rules, items))
}

/// Check every config the run reaches against the catalogue, even when no input is left to
/// lint: the config of each target, of standard input and the one named with `--config`.
/// Returns the warning notice of each unknown rule id.
fn validate_configs(
    request: &LintRequest,
    configs: &Configs<'_>,
    rules: &Catalog,
) -> Result<Vec<Notice>, CliError> {
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
    let mut notices = Vec::new();
    for dir in dirs {
        let found = configs
            .of(&dir)
            .map_err(|e| config_error(&e, &request.root))?;
        if let Some(config) = found
            && seen.insert(config.root.clone())
        {
            notices.extend(lint_settings::validate(&config, rules)?);
        }
    }
    Ok(notices)
}

/// An outcome for a run that could not start.
fn failed(_request: &LintRequest, mut run: Run, error: &CliError) -> RunOutcome {
    // The notice keeps the raw text when the error text was escaped for stderr.
    let notice_message = error.report_message.as_deref().unwrap_or(&error.message);
    // The report must not read as a clean run: a notice marks it unsuccessful.
    run.notices.push(Notice {
        code: KebabName::new(error.code)
            .unwrap_or_else(|_| KebabName::new("lint-failed").expect("lint-failed is kebab-case")),
        level: NoticeLevel::Warning,
        message: NonEmptyString::new(notice_message.to_owned())
            .unwrap_or_else(|_| NonEmptyString::new("lint failed").expect("not empty")),
        path: None,
        data: error.data.clone(),
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
            path: None,
        }),
        (Err(why), _) => run.skip("input-unsupported", shown, format!("{shown}: {why}")),
        (_, Err(_)) => run.skip("input-not-utf8", shown, format!("{shown}: not valid UTF-8")),
    }
    Ok(())
}

/// Read a document found on disk. A symlink was resolved by the caller, so `path` names the
/// target. The read refuses a FIFO or device there instead of blocking on it. On a failure the
/// run records a skip and the result is `None`.
fn read_document(path: &Path, shown: &str, run: &mut Run) -> Option<Vec<u8>> {
    match slopvac_fs::read_capped_regular(
        path,
        slopvac_fs::MAX_DOCUMENT_BYTES,
        slopvac_fs::ReadOptions::new(),
    ) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.input_kind() == Some(slopvac_fs::InputKind::TooLarge) => {
            run.skip(
                "document-too-large",
                shown,
                format!("{shown}: larger than the 64 MiB document limit"),
            );
            None
        }
        Err(_) => {
            run.skip("input-unreadable", shown, format!("{shown}: cannot read"));
            None
        }
    }
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
    let shower = redact::PathShower::new(&request.root);
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
        #[cfg(test)]
        after_discovery::run();
        for skipped in found.skipped {
            let shown = redact::shown_path(&skipped.path, &request.root);
            run.skip(
                "input-unsupported",
                &shown,
                format!("{shown}: {}", skipped.reason),
            );
        }
        for file in found.files {
            let resolved = std::fs::canonicalize(&file.path).ok();
            let canonical = resolved.clone().unwrap_or_else(|| file.path.clone());
            if !seen.insert(canonical.clone()) {
                continue;
            }
            let shown = shower.shown(&file.path, resolved.as_deref());
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
            let Some(bytes) = read_document(&canonical, &shown, run) else {
                continue;
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
                path: Some(canonical),
            });
        }
    }
    Ok(())
}

/// The config that applies to `file` and its path relative to the config root. A file outside
/// that root comes back absolute, so the config's root-relative matching rejects it. The path
/// is for matching only; reports show the separate `shown` path.
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
            .map_or_else(|_| absolute.clone(), Path::to_path_buf);
        (config, rel)
    }))
}

/// One problem in a document's `slopvac:` block, worded without the document's values.
pub(crate) struct BlockError {
    /// The diagnostic's stable code.
    pub(crate) code: &'static str,
    /// `<shown>:<line>:<column>: <key pointer>`, as the document gave the names.
    pub(crate) raw: String,
    /// The same with the file name and the key names escaped for a terminal.
    pub(crate) escaped: String,
}

/// Key paths of the `slopvac:` block that the schema itself names. A key under a map
/// (`rules`, `categories`), an unknown key and an array index come from the document, so a
/// path is shown only up to the last prefix listed here.
const SCHEMA_PATHS: &[&[&str]] = &[
    &["slopvac"],
    &["slopvac", "v"],
    &["slopvac", "genre"],
    &["slopvac", "profile"],
    &["slopvac", "profile", "name"],
    &["slopvac", "profile", "severity_shift"],
    &["slopvac", "locale"],
    &["slopvac", "thresholds"],
    &["slopvac", "thresholds", "max_total_per_100_words"],
    &["slopvac", "thresholds", "max_errors"],
    &["slopvac", "thresholds", "max_warnings"],
    &["slopvac", "thresholds", "min_score"],
    &["slopvac", "thresholds", "max_unicode_dashes"],
    &["slopvac", "categories"],
    &["slopvac", "rules"],
    &["slopvac", "terms"],
    &["slopvac", "terms", "allow"],
];

/// The JSON pointer of `path` cut at the first segment the schema does not own, `/` when no
/// segment is owned.
fn schema_pointer(path: &[String]) -> String {
    let owned = (1..=path.len())
        .take_while(|&n| {
            SCHEMA_PATHS.iter().any(|known| {
                // A directive names the same fields as the block, under its own root.
                let root = if path[0] == "directive" {
                    "directive"
                } else {
                    known[0]
                };
                std::iter::once(root)
                    .chain(known[1..].iter().copied())
                    .eq(path[..n].iter().map(String::as_str))
            })
        })
        .count();
    if owned == 0 {
        return "/".to_owned();
    }
    path[..owned].iter().fold(String::new(), |mut acc, seg| {
        acc.push('/');
        acc.push_str(seg);
        acc
    })
}

/// The problems that `read` found in the block of the document `text`, shown as `shown`. Only
/// the code, the position and the key path are reported: a diagnostic's own message quotes
/// the offending value, and front matter values stay out of error output (policies 1.7).
pub(crate) fn block_errors(shown: &str, text: &str, blocks: &DocumentBlocks) -> Vec<BlockError> {
    [&blocks.front_matter, &blocks.directives]
        .into_iter()
        .flat_map(|read| &read.diagnostics)
        .map(|d| {
            let (line, column) = d.location(text);
            let pointer = schema_pointer(&d.path);
            BlockError {
                code: d.code.as_str(),
                raw: format!("{shown}:{line}:{column}: {pointer}"),
                escaped: format!(
                    "{}:{line}:{column}: {}",
                    slopvac_sanitize::line(shown),
                    slopvac_sanitize::line(&pointer)
                ),
            }
        })
        .collect()
}

/// The `slopvac:` block and the inline directives of the item, read under the project's front
/// matter policy. A top-level `genre` key is ordinary data and never counts.
fn read_block(item: &Item) -> DocumentBlocks {
    let config = item.config.as_ref().map(|(config, _)| &**config);
    read_block_under(&item.text, &item.shown, config)
}

/// Read the `slopvac:` block and the inline directives of `text` under the policy of
/// `config`, or the core defaults when no config applies.
pub(crate) fn read_block_under(
    text: &str,
    shown: &str,
    config: Option<&LoadedConfig>,
) -> DocumentBlocks {
    DocumentBlocks::read(text, shown, config)
}

/// The genre that a document sets, when the policy allows it and the block is valid.
pub(crate) fn declared_genre(blocks: &DocumentBlocks) -> Option<String> {
    blocks.declared_genre()
}

/// A config-relative path as the forward-slash text that `files` patterns match against.
pub(crate) fn rel_text(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Settings and the indexes of the items that share them.
type Groups = BTreeMap<String, (slopvac_engine::Settings, TermInputs, Vec<usize>)>;

/// The engine settings of the file at `rel`, relative to the root of `config`, and the
/// resolved config they came from. `slopvac lint` and the before and after sides of
/// `slopvac migrate` share it.
///
/// # Errors
/// As [`lint_settings::build`].
pub(crate) fn file_settings(
    rules: &Catalog,
    config: &LoadedConfig,
    rel: &Path,
    overrides: &Overrides,
    layers: &DocumentLayers,
) -> Result<(slopvac_engine::Settings, slopvac_config::Resolved), CliError> {
    let resolved = lint_settings::resolve(config, rel, layers);
    let files = config_files(config);
    let mut settings = lint_settings::build(
        rules,
        &resolved,
        lint_settings::config_sources(&files),
        overrides,
    )?;
    settings.genre_assignments = Some(lint_settings::genre_assignments(config));
    Ok((settings, resolved))
}

/// The engine input of `item`, as the lint of the item builds it.
fn input_of<'a>(item: &'a Item, rel: Option<&'a str>, declared: Option<&'a str>) -> Input<'a> {
    let mut input = Input::new(&item.shown, &item.text).with_kind(item.kind);
    if let Some(rel) = rel {
        input = input.with_relative_path(rel);
    }
    if let Some(genre) = declared {
        input = input.with_declared_genre(genre);
    }
    input
}

/// What reading the items once gives, for each item: its read, and the genre the engine
/// chooses for it. Both are `None` for a document that cannot be read.
struct Reads {
    preparsed: Vec<Option<Preparsed>>,
    detected: Vec<Option<String>>,
}

/// Read each item once, and ask the engine of its config for the genre of each.
///
/// The settings of an item follow its genre, and the engine needs the settings, so a probe
/// engine per config reads and detects first. It is built from the settings of the config
/// before any genre layer, which decide neither the genres nor the parse, and it is the only
/// thing in the run that reads these documents: the engines of the groups take the reads.
fn preparse(request: &LintRequest, items: &[Item]) -> Reads {
    let none = LoadedConfig::without_files(request.root.clone());
    let roots = project_roots(request, items);
    let mut reads = Reads {
        preparsed: items.iter().map(|_| None).collect(),
        detected: vec![None; items.len()],
    };
    let mut by_config: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        let key = item.config.as_ref().map(|(c, _)| Arc::as_ptr(c) as usize);
        by_config.entry(key).or_default().push(index);
    }
    for indexes in by_config.into_values() {
        let first = &items[indexes[0]];
        let (config, rel) = match &first.config {
            Some((config, rel)) => (config.as_ref(), rel.clone()),
            None => (&none, PathBuf::from(&first.shown)),
        };
        let probe = Catalog::load().ok().and_then(|catalog| {
            let (settings, _) = file_settings(
                &catalog,
                config,
                &rel,
                &request.overrides,
                &DocumentLayers::default(),
            )
            .ok()?;
            Engine::new(catalog.into_sets(), settings)
                .ok()
                .map(|engine| engine.with_project_roots(roots.clone()))
        });
        let Some(probe) = probe else {
            continue;
        };
        let relatives: Vec<Option<String>> = indexes
            .iter()
            .map(|&i| items[i].config.as_ref().map(|(_, rel)| rel_text(rel)))
            .collect();
        let declared: Vec<Option<String>> = indexes
            .iter()
            .map(|&i| declared_genre(&read_block(&items[i])))
            .collect();
        let inputs: Vec<Input<'_>> = indexes
            .iter()
            .zip(relatives.iter().zip(&declared))
            .map(|(&i, (rel, genre))| input_of(&items[i], rel.as_deref(), genre.as_deref()))
            .collect();
        for ((&i, input), read) in indexes.iter().zip(&inputs).zip(probe.preparse_all(&inputs)) {
            if let Ok(pre) = read {
                reads.detected[i] = probe
                    .detect_genre(input, &pre)
                    .ok()
                    .flatten()
                    .map(|genre| genre.id.as_str().to_owned());
                reads.preparsed[i] = Some(pre);
            }
        }
    }
    reads
}

/// Group the items by their settings: one engine per distinct settings. Each item is read
/// once, to choose the genre its settings follow, and the read comes back for the engine.
fn group_by_settings(
    request: &LintRequest,
    items: &[Item],
    rules: &Catalog,
) -> Result<(Groups, Vec<Option<Preparsed>>), CliError> {
    let mut groups = Groups::new();
    let none = LoadedConfig::without_files(request.root.clone());
    let reads = preparse(request, items);
    for (index, item) in items.iter().enumerate() {
        let (config, rel) = match &item.config {
            Some((config, rel)) => (config.as_ref(), rel.clone()),
            None => (&none, PathBuf::from(&item.shown)),
        };
        let layers = DocumentLayers::read(
            &item.text,
            &item.shown,
            item.config.as_ref().map(|(c, _)| &**c),
        )
        .with_detected_genre(reads.detected[index].clone());
        let (settings, terms, key) =
            settings_for(rules, config, &rel, &request.overrides, &layers)?;
        groups
            .entry(key)
            .or_insert_with(|| (settings, terms, Vec::new()))
            .2
            .push(index);
    }
    Ok((groups, reads.preparsed))
}

/// The engine settings and term inputs for the file at `rel` under `config`, and the key
/// that two files share exactly when one engine serves both. `slopvac lint` and the language
/// server both resolve a file's settings here.
pub(crate) fn settings_for(
    rules: &Catalog,
    config: &LoadedConfig,
    rel: &Path,
    cli: &Overrides,
    layers: &DocumentLayers,
) -> Result<(slopvac_engine::Settings, TermInputs, String), CliError> {
    let (settings, resolved) = file_settings(rules, config, rel, cli, layers)?;
    let terms = lint_terms::inputs(&resolved, &config.root);
    let key = format!("{settings:?}{terms:?}");
    Ok((settings, terms, key))
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

/// What a fix run needs besides the items: the engine of the group and, for each input of
/// the group, the lint context `lint_items` gives the engine.
struct FixContext<'a> {
    /// The root handle opened before the files were collected.
    root: &'a std::io::Result<Root>,
    engine: &'a Engine,
    relatives: &'a [Option<String>],
    declared: &'a [Option<String>],
}

/// The entries of `values` whose flag in `keep` is set.
fn retain<T: Clone>(values: &[T], keep: &[bool]) -> Vec<T> {
    values
        .iter()
        .zip(keep)
        .filter(|(_, keep)| **keep)
        .map(|(value, _)| value.clone())
        .collect()
}

/// The reason for an I/O error: the typed causes the engine raises get their catalog text,
/// anything else the system's own wording.
fn io_reason(source: &std::io::Error) -> LintFixReason {
    let cause = source.get_ref();
    if cause.is_some_and(<dyn std::error::Error + Send + Sync>::is::<AclCheckFailed>) {
        LintFixReason::AclCheck
    } else if cause.is_some_and(<dyn std::error::Error + Send + Sync>::is::<RootChanged>) {
        LintFixReason::RootChanged
    } else {
        LintFixReason::Io {
            detail: source.to_string(),
        }
    }
}

/// The reason text for a fix that failed for a cause other than a refusal or a lint error.
fn fix_failure_reason(err: FixError) -> String {
    let reason = match err {
        FixError::NotUtf8(_) => LintFixReason::NotUtf8,
        FixError::Io { source, .. } => io_reason(&source),
        other => LintFixReason::Io {
            detail: other.to_string(),
        },
    };
    slopvac_messages::render(&reason)
}

/// Write the safe replacements of each file in `indexes` and swap the text of the item for
/// the fixed text, so the report that follows describes the file as it is now.
///
/// Returns, for each index, whether the file is still to be reported: a file the engine
/// refused to write is recorded as not checked (exit 2) and left out. Standard input has no
/// file and is never written.
fn fix_group(
    context: &FixContext<'_>,
    indexes: &[usize],
    items: &mut [Item],
    run: &mut Run,
) -> Vec<bool> {
    let mut keep = Vec::with_capacity(indexes.len());
    for (position, &index) in indexes.iter().enumerate() {
        let item = &mut items[index];
        let Some(path) = item.path.clone() else {
            keep.push(true);
            continue;
        };
        let file_context = FileContext {
            kind: item.kind,
            shown: Some(&item.shown),
            relative_path: context.relatives[position].as_deref(),
            declared_genre: context.declared[position].as_deref(),
        };
        let root = match context.root {
            Ok(root) => root,
            Err(err) => {
                let shown = item.shown.clone();
                let failed = LintFixMessage::Failed {
                    shown: shown.clone(),
                    reason: slopvac_messages::render(&io_reason(err)),
                };
                run.skip(failed.code(), &shown, slopvac_messages::render(&failed));
                keep.push(false);
                continue;
            }
        };
        let fixed = fix_file_in(
            context.engine,
            root,
            &path,
            &file_context,
            false,
            None,
            &mut |_| Ok(()),
        );
        match fixed {
            Ok(FileFix::Done(result)) => {
                // The fix read the file again, so its text is the one to report even when
                // nothing was written.
                item.text = result.report.text;
                keep.push(true);
            }
            Ok(FileFix::Refused(refusal)) => {
                let shown = item.shown.clone();
                let message = slopvac_messages::render(&LintFixMessage::Refused {
                    shown: shown.clone(),
                    reason: refusal.code().to_owned(),
                });
                run.skip(refusal.code(), &shown, message);
                keep.push(false);
            }
            Err(FixError::Engine(err)) => {
                run.skip_engine_error(&item.shown, &err);
                keep.push(false);
            }
            Err(err) => {
                let shown = item.shown.clone();
                let failed = LintFixMessage::Failed {
                    shown: shown.clone(),
                    reason: fix_failure_reason(err),
                };
                run.skip(failed.code(), &shown, slopvac_messages::render(&failed));
                keep.push(false);
            }
        }
    }
    keep
}

fn lint_items(
    request: &LintRequest,
    root: Option<&std::io::Result<Root>>,
    mut items: Vec<Item>,
    rules: Catalog,
    mut run: Run,
) -> RunOutcome {
    let (groups, mut preparsed) = match group_by_settings(request, &items, &rules) {
        Ok(found) => found,
        Err(err) => return failed(request, run, &err),
    };

    // A fix changes the text, so what was read of it no longer holds.
    if root.is_some() {
        preparsed.fill_with(|| None);
    }
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
        let relatives: Vec<Option<String>> = indexes
            .iter()
            .map(|&i| items[i].config.as_ref().map(|(_, rel)| rel_text(rel)))
            .collect();
        let (indexes, relatives) = if let Some(root) = root {
            // The fix needs the genre each file declares before it is fixed. The report
            // below reads the front matter again from the text that is left.
            let declared: Vec<Option<String>> = indexes
                .iter()
                .map(|&i| declared_genre(&read_block(&items[i])))
                .collect();
            let context = FixContext {
                root,
                engine: &engine,
                relatives: &relatives,
                declared: &declared,
            };
            let keep = fix_group(&context, &indexes, &mut items, &mut run);
            (retain(&indexes, &keep), retain(&relatives, &keep))
        } else {
            (indexes, relatives)
        };
        let reads: Vec<DocumentBlocks> = indexes.iter().map(|&i| read_block(&items[i])).collect();
        for (&i, read) in indexes.iter().zip(&reads) {
            run.front_matter_errors(&items[i], read);
        }
        let declared: Vec<Option<String>> =
            reads.iter().map(DocumentBlocks::declared_genre).collect();
        let inputs: Vec<Input<'_>> = indexes
            .iter()
            .zip(relatives.iter().zip(&declared))
            .map(|(&i, (rel, genre))| {
                let input = input_of(&items[i], rel.as_deref(), genre.as_deref());
                match &preparsed[i] {
                    Some(pre) => input.with_preparsed(pre),
                    None => input,
                }
            })
            .collect();
        for (&index, result) in indexes.iter().zip(engine.lint_all_with_notices(&inputs)) {
            match result {
                Ok((document, notices)) => {
                    // Extraction notices of unsupported content: shown, never a failure.
                    run.notices.extend(notices);
                    documents[index] = Some(document);
                }
                Err(err) => {
                    run.skip_engine_error(&items[index].shown, &err);
                }
            }
        }
    }

    let documents: Vec<Document> = documents.into_iter().flatten().collect();
    for document in &documents {
        for notice in &document.unchecked {
            if notice.level == NoticeLevel::Warning {
                // Part of the check did not run, so the result cannot establish a pass.
                // The notice keeps raw text for the report formats. The error reaches stderr, so
                // it gets the project root redacted and the whole text escaped once.
                let raw = format!(
                    "{}: {}: {}",
                    document.path,
                    notice.code.as_str(),
                    notice.message
                );
                run.incomplete.push(CliError::new(
                    "check-incomplete",
                    line(&redact::redact_root(&raw, &request.root)),
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

/// The display name of each config file and whether it was pulled in by `extends`.
pub(crate) fn config_files(config: &LoadedConfig) -> Vec<(String, bool)> {
    config
        .files
        .iter()
        .map(|f| {
            (
                f.display.clone(),
                f.layer == slopvac_model::ConfigLayer::Extends,
            )
        })
        .collect()
}

/// A test seam: code that runs once discovery has listed the files and before they are read,
/// so a test can change a file in that window deterministically.
#[cfg(test)]
mod after_discovery {
    use std::cell::RefCell;

    type Hook = Box<dyn FnOnce()>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(super) fn set(hook: Hook) {
        HOOK.with(|h| *h.borrow_mut() = Some(hook));
    }

    pub(super) fn run() {
        if let Some(hook) = HOOK.with(|h| h.borrow_mut().take()) {
            hook();
        }
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;
    use crate::lint_settings::Overrides;

    fn request(root: &Path, target: &Path) -> LintRequest {
        LintRequest {
            root: root.to_path_buf(),
            targets: vec![target.to_path_buf()],
            stdin: None,
            stdin_plan: None,
            config: None,
            code_comments: false,
            overrides: Overrides::default(),
            fix: false,
        }
    }

    /// Run the public linter on a worker thread, with `hook` installed on that thread. When it
    /// blocks, the FIFO at `fifo` is opened without blocking to wake it, and the test fails.
    fn lint_within_timeout(
        fifo: &Path,
        request: LintRequest,
        hook: Option<Box<dyn FnOnce() + Send>>,
    ) -> RunOutcome {
        let probe = fifo.to_path_buf();
        within_timeout(&probe, move || {
            if let Some(hook) = hook {
                after_discovery::set(hook);
            }
            EngineLinter.lint(&request)
        })
    }

    /// Run `f` on a worker thread and return its result. A panic re-raises at once; a worker
    /// that has not finished after five seconds gets the FIFO's write end opened without
    /// blocking, so a reader stuck in `open` wakes, and the test then fails.
    fn within_timeout<T: Send + 'static>(fifo: &Path, f: impl FnOnce() -> T + Send + 'static) -> T {
        use std::os::unix::fs::OpenOptionsExt;
        use std::sync::mpsc::{self, RecvTimeoutError};
        use std::time::Duration;

        const O_NONBLOCK: i32 = if cfg!(target_os = "linux") {
            0o4000
        } else {
            0x4
        };
        const LIMIT: Duration = Duration::from_secs(5);

        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        match rx.recv_timeout(LIMIT) {
            Ok(value) => {
                worker.join().expect("the worker finished");
                value
            }
            Err(RecvTimeoutError::Disconnected) => match worker.join() {
                Err(panic) => std::panic::resume_unwind(panic),
                Ok(()) => panic!("the worker ended without a result"),
            },
            Err(RecvTimeoutError::Timeout) => {
                // ENXIO means no reader is waiting, so there is nothing to wake.
                let _writer = std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(O_NONBLOCK)
                    .open(fifo);
                match rx.recv_timeout(LIMIT) {
                    Ok(_) | Err(RecvTimeoutError::Disconnected) => {
                        let _ = worker.join();
                        panic!("the read blocked on a FIFO until it was woken");
                    }
                    Err(RecvTimeoutError::Timeout) => panic!("the read blocked on a FIFO"),
                }
            }
        }
    }

    /// The read refusal escapes the file name: the name shows as `\u{..}` text, never raw.
    #[rstest::rstest]
    #[case::esc("a\u{1b}]0;x\u{7}b.md", "a\\u{1b}]0;x\\u{7}b.md")]
    #[case::newline("a\nb.md", "a\\u{a}b.md")]
    #[case::bidi("a\u{202e}b.md", "a\\u{202e}b.md")]
    fn an_unreadable_file_is_reported_with_an_escaped_name(
        #[case] name: &str,
        #[case] escaped: &str,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let doc = root.join(name);
        std::fs::write(&doc, "Hello there.\n").unwrap();
        let swap = doc.clone();
        let outcome = lint_within_timeout(
            &doc,
            request(&root, &doc),
            Some(Box::new(move || {
                std::fs::remove_file(&swap).unwrap();
                assert!(
                    std::process::Command::new("mkfifo")
                        .arg(&swap)
                        .status()
                        .unwrap()
                        .success()
                );
            })),
        );
        let messages: Vec<&str> = outcome
            .incomplete
            .iter()
            .map(|e| e.message.as_str())
            .collect();
        assert_eq!(messages, [format!("{escaped}: cannot read")]);
    }

    /// Discovery lists a regular file; the file becomes a FIFO before the read. The run must
    /// report it as unreadable and finish.
    #[test]
    fn a_file_that_becomes_a_fifo_after_discovery_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let doc = root.join("a.md");
        std::fs::write(&doc, "Hello there.\n").unwrap();
        let swap = doc.clone();
        let outcome = lint_within_timeout(
            &doc,
            request(&root, &doc),
            Some(Box::new(move || {
                std::fs::remove_file(&swap).unwrap();
                assert!(
                    std::process::Command::new("mkfifo")
                        .arg(&swap)
                        .status()
                        .unwrap()
                        .success()
                );
            })),
        );
        let codes: Vec<&str> = outcome.incomplete.iter().map(|e| e.code).collect();
        assert_eq!(codes, ["input-unreadable"]);
        assert_eq!(outcome.documents, 0);
    }

    #[test]
    fn a_regular_file_is_linted() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let doc = root.join("a.md");
        std::fs::write(&doc, "Hello there.\n").unwrap();
        let outcome = lint_within_timeout(&doc, request(&root, &doc), None);
        assert_eq!(outcome.incomplete.len(), 0);
        assert_eq!(outcome.documents, 1);
    }

    /// A target that is a symlink to a regular file is still linted.
    #[test]
    fn a_symlink_to_a_regular_file_is_linted() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let real = root.join("real.md");
        std::fs::write(&real, "Hello there.\n").unwrap();
        let link = root.join("link.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let outcome = lint_within_timeout(&link, request(&root, &link), None);
        assert_eq!(outcome.incomplete.len(), 0);
        assert_eq!(outcome.documents, 1);
    }

    /// A target reached through a symlink to an external directory, which vanishes after
    /// discovery, cannot be canonicalized. Its notice must not show the external directory.
    #[test]
    fn a_vanished_file_behind_an_external_symlink_hides_the_external_directory() {
        let dir = tempfile::tempdir().unwrap();
        let top = std::fs::canonicalize(dir.path()).unwrap();
        let root = top.join("proj");
        let external = top.join("elsewhere").join("sentinel-private");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("a.md"), "Hello there.\n").unwrap();
        std::os::unix::fs::symlink(top.join("elsewhere"), root.join("link")).unwrap();
        let target = root.join("link").join("sentinel-private").join("a.md");
        let gone = external.join("a.md");
        let outcome = lint_within_timeout(
            &root.join("probe-fifo"),
            request(&root, &target),
            Some(Box::new(move || std::fs::remove_file(&gone).unwrap())),
        );
        let text = format!("{:?}", outcome.incomplete);
        assert!(text.contains("<external>/a.md"), "{text}");
        assert!(!text.contains("sentinel-private"), "{text}");
    }
}

#[cfg(test)]
mod skip_tests {
    use super::*;
    use rstest::rstest;
    use slopvac_model::PositionError;
    use slopvac_parse::ParseError;
    use slopvac_pcre2::MatchError;
    use slopvac_text::TextError;

    /// Every `EngineError` variant. The enum is `#[non_exhaustive]` across crates, so the
    /// compiler cannot force this list to grow: add a case when the engine adds a variant.
    #[rstest]
    #[case::parse(EngineError::from(ParseError::Mdx("x".into())), "lint-failed")]
    #[case::text(
        EngineError::from(TextError::from(MatchError::MatchLimit { limit: 1 })),
        "lint-failed"
    )]
    #[case::context(EngineError::Context("x".into()), "lint-failed")]
    #[case::position(EngineError::from(PositionError::PastEnd(9)), "lint-failed")]
    #[case::bad_range(EngineError::BadRange("x".into()), "lint-failed")]
    #[case::tag_lookup_budget(
        EngineError::TagLookupBudget { limit: 16_384 },
        "context-budget-exceeded"
    )]
    #[case::tag_build_budget(
        EngineError::TagBuildBudget { limit: 65_536 },
        "context-budget-exceeded"
    )]
    #[case::terms(EngineError::Terms("x".into()), "lint-failed")]
    #[case::internal(EngineError::Internal("x".into()), "lint-failed")]
    fn an_engine_error_leaves_the_file_unchecked_and_the_run_incomplete(
        #[case] err: EngineError,
        #[case] code: &str,
    ) {
        let mut run = Run::default();
        run.skip_engine_error("docs/a.md", &err);

        let codes: Vec<&str> = run.incomplete.iter().map(|e| e.code).collect();
        assert_eq!(codes, [code]);
        assert_eq!(run.notices.len(), 1);
        assert_eq!(run.notices[0].code.as_str(), code);
        assert_eq!(run.notices[0].level, NoticeLevel::Warning);
        assert_eq!(run.notices[0].path.as_deref(), Some("docs/a.md"));
    }

    /// A context failure holds a pattern with a backslash and a newline. The engine error keeps
    /// the raw text: the notice carries it as it is, and stderr gets it escaped once.
    #[test]
    fn a_context_failure_is_raw_in_the_notice_and_escaped_once_on_stderr() {
        let err = EngineError::Context("pattern `\\d+\n` failed while matching".into());
        let mut run = Run::default();
        run.skip_engine_error("docs/a.md", &err);

        assert_eq!(
            run.notices[0].message.as_str(),
            "docs/a.md: context tagger failed: pattern `\\d+\n` failed while matching"
        );
        assert_eq!(
            run.incomplete[0].message,
            "docs/a.md: context tagger failed: pattern `\\\\d+\\u{a}` failed while matching"
        );
    }
}

#[cfg(test)]
mod fix_tests {
    use super::*;
    use slopvac_engine::Settings;
    use slopvac_model::{GenreId, GenreResult, GenreSource};

    #[test]
    fn the_report_uses_the_text_the_fix_read_even_when_it_wrote_nothing() {
        // The file changed after the run read it. The fix read the new text and found
        // nothing to write, so the report must still describe the new text.
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().canonicalize().expect("canonical root");
        let path = root.join("a.md");
        std::fs::write(&path, "# New\n").expect("write");
        let mut items = vec![Item {
            shown: "a.md".to_owned(),
            text: "# Old\n".to_owned(),
            kind: Kind::Markdown,
            config: None,
            dir: Some(root.clone()),
            path: Some(path),
        }];
        let request = LintRequest {
            root,
            targets: Vec::new(),
            stdin: None,
            stdin_plan: None,
            config: None,
            code_comments: false,
            overrides: crate::lint_settings::Overrides::default(),
            fix: true,
        };
        let engine = Engine::new(
            Vec::new(),
            Settings::new(
                NonEmptyString::new("normal").expect("profile"),
                GenreResult {
                    id: GenreId::new("reference").expect("genre"),
                    source: GenreSource::Default,
                    reasons: Vec::new(),
                },
            ),
        )
        .expect("engine");
        let held = Root::open(&request.root);
        let context = FixContext {
            root: &held,
            engine: &engine,
            relatives: &[None],
            declared: &[None],
        };
        let mut run = Run::default();

        let keep = fix_group(&context, &[0], &mut items, &mut run);

        assert_eq!(keep, [true]);
        assert_eq!(items[0].text, "# New\n");
    }
    #[test]
    fn a_fix_that_failed_gives_a_reason_for_each_kind_of_failure() {
        let io = |source| FixError::Io {
            path: "a.md".to_owned(),
            source,
        };
        assert_eq!(
            fix_failure_reason(FixError::NotUtf8("a.md".to_owned())),
            "not valid UTF-8"
        );
        assert_eq!(
            fix_failure_reason(io(std::io::Error::other(AclCheckFailed))),
            "the access control list check failed"
        );
        assert_eq!(
            fix_failure_reason(io(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied
            ))),
            "permission denied"
        );
    }

    /// An engine with no rules, for tests that never lint.
    fn bare_engine() -> Engine {
        Engine::new(
            Vec::new(),
            Settings::new(
                NonEmptyString::new("normal").expect("profile"),
                GenreResult {
                    id: GenreId::new("reference").expect("genre"),
                    source: GenreSource::Default,
                    reasons: Vec::new(),
                },
            ),
        )
        .expect("engine")
    }

    #[cfg(unix)]
    #[test]
    fn a_root_replaced_while_it_was_opened_is_reported_with_the_registry_reason() {
        let parent = tempfile::tempdir().expect("tempdir");
        let root = parent.path().join("root");
        std::fs::create_dir(&root).expect("root");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let held = Root::open_hooked(&root, &mut || {
            std::fs::rename(&root, parent.path().join("moved")).expect("move the root");
            std::os::unix::fs::symlink(elsewhere.path(), &root).expect("link in its place");
        });
        let mut items = vec![Item {
            shown: "a.md".to_owned(),
            text: "# Old\n".to_owned(),
            kind: Kind::Markdown,
            config: None,
            dir: Some(root.clone()),
            path: Some(root.join("a.md")),
        }];
        let engine = bare_engine();
        let context = FixContext {
            root: &held,
            engine: &engine,
            relatives: &[None],
            declared: &[None],
        };
        let mut run = Run::default();

        let keep = fix_group(&context, &[0], &mut items, &mut run);

        assert_eq!(keep, [false]);
        let notice = run.notices.first().expect("a notice");
        assert_eq!(notice.code.as_str(), "fix-failed");
        assert_eq!(
            notice.message.as_str(),
            "a.md: not fixed: the project root was replaced while it was opened"
        );
    }
}
