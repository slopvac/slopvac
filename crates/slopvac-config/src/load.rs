//! Read a `slopvac.toml` and the configs it extends into an ordered list of files.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::Value as Json;
use slopvac_model::ConfigLayer;

use crate::diag::{BudgetData, Code, ConfigBudget, Diagnostic};
use crate::error::{ConfigError, Rejected};
use crate::files::{FilesError, FilesMatcher, is_under_root};
use crate::flatten::{Origin, flatten};
use crate::messages::{ConfigPhrase, ConfigText};
use crate::presets::Presets;
use crate::read::{
    MAX_CONFIG_BYTES, MAX_EXTENDS_APPLIED_BYTES, MAX_EXTENDS_BYTES, MAX_EXTENDS_DEPTH,
    MAX_EXTENDS_FILES, MAX_FILE_BLOCKS, MAX_INDEX_EXTRA, ReadFailure, read_text,
};
use crate::schema::validate_config;
use crate::source::Setting;
use crate::tree::{Node, Value, parse_toml};
use crate::version::check_version;

/// Which table a block came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// An `[[overrides]]` block.
    Override,
    /// A `[[genres]]` block.
    Genre,
}

/// A glob-scoped patch.
#[derive(Debug, Clone)]
pub struct Block {
    /// Position in its table, from zero.
    pub index: usize,
    /// Which table it came from.
    pub kind: BlockKind,
    /// The `files` patterns, as written.
    pub files: Vec<String>,
    /// The settings it patches in, in layer `override`.
    pub settings: Vec<Setting>,
    order: usize,
    matcher: FilesMatcher,
}

impl Block {
    /// Whether the block applies to `rel`, a path relative to the project root.
    #[must_use]
    pub fn matches(&self, rel: &Path) -> bool {
        self.matcher.is_match(rel)
    }

    /// The genre this block assigns, if it is a `[[genres]]` block.
    #[must_use]
    pub fn genre(&self) -> Option<&str> {
        if self.kind != BlockKind::Genre {
            return None;
        }
        self.settings
            .iter()
            .find(|s| s.path == ["genre"])
            .and_then(|s| s.value.as_str())
    }
}

/// One parsed, validated config file.
#[derive(Debug, Clone)]
pub struct ConfigFile {
    /// Path shown in provenance.
    pub display: String,
    /// `extends` for an extended file, `project` for the nearest config.
    pub layer: ConfigLayer,
    /// The whole-file settings, outside `overrides` and `genres`.
    pub settings: Vec<Setting>,
    /// `[[genres]]` blocks, then `[[overrides]]` blocks, each in file order.
    pub blocks: Vec<Block>,
}

/// The nearest config of a project with everything it extends.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    /// Directory of the nearest config. Globs in every file match from here.
    pub root: PathBuf,
    /// Extended files first, in the order they apply, and the project config last.
    pub files: Vec<Arc<ConfigFile>>,
    dependencies: Vec<PathBuf>,
    exclude: Gitignore,
}

impl LoadedConfig {
    /// Every file the load read, the config itself first and then each extended file in the
    /// order the load met it. The paths are absolute and lexically normalised, with no `..`,
    /// wherever the file sits relative to the root. A watcher compares them by equality.
    #[must_use]
    pub fn dependencies(&self) -> &[PathBuf] {
        &self.dependencies
    }
    /// The final value of a whole-run key, from core defaults up through the config files.
    #[must_use]
    pub fn project_value(&self, path: &[&str]) -> Option<Json> {
        self.files
            .iter()
            .flat_map(|f| f.settings.iter())
            .rfind(|s| s.path == path)
            .map(|s| s.value.clone())
            .or_else(|| {
                crate::resolve::core_defaults()
                    .into_iter()
                    .rfind(|s| s.path == path)
                    .map(|s| s.value)
            })
    }

    /// The `[[genres]]` blocks of every file as `(files, genre)` pairs, in the order they
    /// apply: extended files first, each file in block order. The last block whose `files`
    /// match a path assigns its genre, as in [`LoadedConfig::resolve`].
    #[must_use]
    pub fn genre_assignments(&self) -> Vec<(&[String], &str)> {
        self.files
            .iter()
            .flat_map(|f| f.blocks.iter())
            .filter_map(|b| Some((b.files.as_slice(), b.genre()?)))
            .collect()
    }

    /// A config with no files, for a project that has none: only the core defaults apply and
    /// nothing is excluded. `root` is the directory that stands in for the project root.
    #[must_use]
    pub fn without_files(root: PathBuf) -> Self {
        Self {
            root,
            files: Vec::new(),
            dependencies: Vec::new(),
            exclude: Gitignore::empty(),
        }
    }

    /// Whether `rel`, a path relative to the project root, is outside the lint run. A path
    /// outside the root is never excluded.
    #[must_use]
    pub fn is_excluded(&self, rel: &Path) -> bool {
        is_under_root(rel)
            && self
                .exclude
                .matched_path_or_any_parents(rel, false)
                .is_ignore()
    }
}

/// Load the config at `path` and everything it extends.
pub fn load(path: &Path, presets: &dyn Presets) -> Result<LoadedConfig, ConfigError> {
    load_labeled(path, &path.display().to_string(), presets)
}

/// Load the config at `path`, naming it `label` in errors. Use it when `path` is an absolute
/// form of what the user typed, so a failure shows the user's own words.
pub fn load_labeled(
    path: &Path,
    label: &str,
    presets: &dyn Presets,
) -> Result<LoadedConfig, ConfigError> {
    load_counted(path, label, presets).map(|(config, _)| config)
}

/// Load `text` as the config at `path`, and everything it extends, naming it `label` in
/// errors. `path` says where the file would sit, and only its directory must exist. Use it
/// for a config that is not on disk yet, such as the one `slopvac migrate` would write.
///
/// # Errors
/// As [`load_labeled`]: a `config-too-large` error when `text` is over
/// [`crate::MAX_CONFIG_BYTES`], and a `config-io` error when the directory of `path` cannot
/// be resolved.
pub fn load_text(
    path: &Path,
    label: &str,
    text: String,
    presets: &dyn Presets,
) -> Result<LoadedConfig, ConfigError> {
    if text.len() as u64 > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge {
            path: label.to_owned(),
            cap: MAX_CONFIG_BYTES,
            file: None,
        });
    }
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = fs::canonicalize(dir.unwrap_or_else(|| Path::new(".")))
        .map_err(|source| ConfigError::io(label, source))?;
    let abs = match path.file_name() {
        Some(name) => dir.join(name),
        None => dir.join(crate::discover::FILE_NAME),
    };
    load_from(&abs, text, presets).map(|(config, _)| config)
}

/// As [`load_labeled`], also returning how many files were read.
fn load_counted(
    path: &Path,
    label: &str,
    presets: &dyn Presets,
) -> Result<(LoadedConfig, usize), ConfigError> {
    let abs = fs::canonicalize(path).map_err(|source| ConfigError::io(label, source))?;
    let text = ConfigError::read_config(&abs, label)?;
    load_from(&abs, text, presets)
}

/// Load `text` as the config at the absolute path `abs`, and everything it extends.
fn load_from(
    abs: &Path,
    text: String,
    presets: &dyn Presets,
) -> Result<(LoadedConfig, usize), ConfigError> {
    let root = abs.parent().map_or_else(PathBuf::new, Path::to_path_buf);
    let mut loader = Loader {
        root: &root,
        presets,
        stack: Vec::new(),
        files: Vec::new(),
        expanded: HashMap::new(),
        applied_bytes: 0,
        deepest: 0,
        bytes: text.len() as u64,
        reads: 1,
        attempted: vec![abs.to_path_buf()],
    };
    let outcome = loader.load_text(
        &Key::Path(abs.to_path_buf()),
        &relative_display(abs, &root),
        text,
        ConfigLayer::Project,
    );
    if let Err(error) = outcome {
        return Err(error.with_reads(loader.attempted));
    }
    let exclude = build_exclude(&loader.files, &root);
    let reads = loader.reads;
    let dependencies = loader.attempted;
    let files = loader.files.into_iter().map(|a| a.file).collect();
    Ok((
        LoadedConfig {
            root,
            files,
            dependencies,
            exclude,
        },
        reads,
    ))
}

fn build_exclude(files: &[Applied], root: &Path) -> Gitignore {
    let patterns = files
        .iter()
        .flat_map(|a| a.file.settings.iter())
        .rfind(|s| s.path == ["exclude"])
        .map_or_else(
            || {
                crate::resolve::core_defaults()
                    .into_iter()
                    .find(|s| s.path == ["exclude"])
                    .map(|s| s.value)
            },
            |s| Some(s.value.clone()),
        );
    let mut builder = GitignoreBuilder::new(root);
    for p in patterns
        .iter()
        .filter_map(Json::as_array)
        .flatten()
        .filter_map(Json::as_str)
    {
        // Patterns were validated as non-empty strings; a glob the matcher cannot compile is skipped
        // here and reported against its block or file by `compile`.
        let _ = builder.add_line(None, p);
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// `name` joined to `dir` and lexically normalised.
pub(crate) fn normalise_join(dir: &Path, name: &str) -> PathBuf {
    normalise(&dir.join(name))
}

/// Lexically normalise `path`, as every other part of slopvac does, so a path that a watcher
/// compares is spelled the one way.
fn normalise(path: &Path) -> PathBuf {
    slopvac_fs::lexical_normalize(path)
}

/// Whether two path components name the same place. A verbatim prefix (`\\?\C:`) and its plain
/// form (`C:`) are the same drive, and `Path` equality treats them as different.
fn same_component(a: Component<'_>, b: Component<'_>) -> bool {
    use std::path::Prefix::{Disk, UNC, VerbatimDisk, VerbatimUNC};
    let (Component::Prefix(a), Component::Prefix(b)) = (a, b) else {
        return a == b;
    };
    match (a.kind(), b.kind()) {
        (Disk(x) | VerbatimDisk(x), Disk(y) | VerbatimDisk(y)) => x.eq_ignore_ascii_case(&y),
        (UNC(s1, h1) | VerbatimUNC(s1, h1), UNC(s2, h2) | VerbatimUNC(s2, h2)) => {
            s1.eq_ignore_ascii_case(s2) && h1.eq_ignore_ascii_case(h2)
        }
        (x, y) => x == y,
    }
}

/// The path of `file` relative to `root`, with `..` for each directory it climbs out of.
///
/// A file that shares no prefix with the root, such as one on another drive or UNC share, has no
/// relative form. It shows as `<external>/<file name>`, the form policies.md section 1.7 gives to
/// a path outside the root, so a report never carries an absolute path.
fn relative_display(file: &Path, root: &Path) -> String {
    let file = normalise(file);
    let f: Vec<_> = file.components().collect();
    let r: Vec<_> = root.components().collect();
    let common = f
        .iter()
        .zip(&r)
        .take_while(|(a, b)| same_component(**a, **b))
        .count();
    if common == 0 {
        let name = file
            .file_name()
            .map_or_else(String::new, |n| format!("/{}", n.to_string_lossy()));
        return format!("<external>{name}");
    }
    // `/` on every platform, so provenance reads the same in reports on Windows.
    let mut parts: Vec<String> = vec!["..".to_owned(); r.len() - common];
    parts.extend(
        f[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Path(PathBuf),
    Pack(String),
}

/// One application of a parsed config file. The file is shared, so an application beyond the
/// first copies a pointer and not the settings.
#[derive(Clone)]
struct Applied {
    file: Arc<ConfigFile>,
    /// Bytes of the file's text, for the applied-bytes budget.
    bytes: u64,
}

/// What an extended file expanded to, kept so a later reference copies pointers only.
#[derive(Clone)]
struct Expansion {
    /// The applications in order, the file's own last.
    files: Vec<Applied>,
    /// Levels the subtree spans, the file itself included: a file with no `extends` is 1.
    height: usize,
}

/// An `extends` entry that is refused: the diagnostic code, message, help and any budget detail.
struct Refusal {
    code: Code,
    message: String,
    help: Option<String>,
    data: Option<Box<BudgetData>>,
}

impl Refusal {
    fn new(code: Code, message: String, help: Option<String>) -> Self {
        Self {
            code,
            message,
            help,
            data: None,
        }
    }
}

struct Loader<'a> {
    root: &'a Path,
    presets: &'a dyn Presets,
    stack: Vec<(Key, String)>,
    files: Vec<Applied>,
    /// For each extended file already loaded, the files it expanded to, in apply order. A file
    /// that several others extend is read and parsed once and its expansion is copied after that.
    expanded: HashMap<Key, Expansion>,
    /// The deepest level reached so far in the subtree being loaded.
    deepest: usize,
    /// Bytes of text of the extended files applied or reserved so far, one count per application.
    applied_bytes: u64,
    /// Bytes read so far from distinct reads.
    bytes: u64,
    /// Files read so far.
    reads: usize,
    /// Every path the load reached, in the order it met them, each once. A path that cannot be
    /// read is on the list too.
    attempted: Vec<PathBuf>,
}

/// The file a key names, when it names one.
fn key_path(key: &Key) -> Option<PathBuf> {
    match key {
        Key::Path(p) => Some(p.clone()),
        Key::Pack(_) => None,
    }
}

impl Loader<'_> {
    fn load_text(
        &mut self,
        key: &Key,
        display: &str,
        text: String,
        layer: ConfigLayer,
    ) -> Result<(), ConfigError> {
        let label = self.error_label(key, display);
        let reject = |text: String, diagnostics: Vec<Diagnostic>| {
            ConfigError::Rejected(Box::new(Rejected {
                file: label.clone(),
                path: key_path(key),
                reads: Vec::new(),
                text,
                diagnostics,
            }))
        };
        let tree = match parse_toml(&text) {
            Ok(t) => t,
            Err(e) => {
                let span = e.span.unwrap_or(0..0);
                let d = Diagnostic {
                    code: Code::Parse,
                    message: ConfigText::NotValidToml { detail: e.message }.to_string(),
                    label: ConfigPhrase::LabelParserStopped.text(),
                    span,
                    help: None,
                    path: Vec::new(),
                    data: None,
                };
                return Err(reject(text, vec![d]));
            }
        };
        if let Err(d) = check_version(&tree, &text) {
            return Err(reject(text, vec![d]));
        }
        if let Some(d) = self.check_block_count(&tree, key, display) {
            return Err(reject(text, vec![d]));
        }
        if let Some(d) = self.check_index_count(&tree, key, display) {
            return Err(reject(text, vec![d]));
        }
        let mut diagnostics = validate_config(&tree, &text);
        if diagnostics.is_empty() {
            check_exclude(&tree, &mut diagnostics);
            self.check_styles(&tree, &mut diagnostics);
        }
        let blocks = if diagnostics.is_empty() {
            self.blocks(&tree, display, &text, &mut diagnostics)
        } else {
            Vec::new()
        };
        if !diagnostics.is_empty() {
            return Err(reject(text, diagnostics));
        }
        self.stack.push((key.clone(), display.to_owned()));
        let extended = self.extend(&tree, key, &text, display);
        self.stack.pop();
        extended?;
        let settings = flatten(
            &tree,
            Origin {
                layer,
                file: display,
                text: &text,
            },
            &["schema_version", "extends", "overrides", "genres"],
        );
        self.files.push(Applied {
            file: Arc::new(ConfigFile {
                display: display.to_owned(),
                layer,
                settings,
                blocks,
            }),
            bytes: text.len() as u64,
        });
        Ok(())
    }

    /// How a file is named in an error: relative to the project root when it sits under it,
    /// else `<external>/<file name>`. Provenance keeps the fuller relative form; an error
    /// must not show the directories beside or above the root (policies.md, 1.7).
    fn error_label(&self, key: &Key, display: &str) -> String {
        match key {
            Key::Path(p) => safe_display(p, self.root),
            Key::Pack(_) => display.to_owned(),
        }
    }

    /// Refuse a file that lists more than [`MAX_INDEX_EXTRA`] `[[index.extra]]` entries, before
    /// the schema check, the duplicate-name scan or any fetch.
    fn check_index_count(&self, tree: &Node, key: &Key, display: &str) -> Option<Diagnostic> {
        let extra = tree.get("index").and_then(|index| index.get("extra"))?;
        let Value::Array(items) = &extra.value else {
            return None;
        };
        if items.len() <= MAX_INDEX_EXTRA {
            return None;
        }
        let refusal = self.over_budget(key, display, ConfigBudget::IndexExtra);
        Some(Diagnostic {
            code: refusal.code,
            message: refusal.message,
            label: ConfigPhrase::LabelTooManyIndexes.text(),
            span: items.first().map_or(0..0, |first| first.span.clone()),
            help: refusal.help,
            path: vec!["index".to_owned(), "extra".to_owned()],
            data: refusal.data,
        })
    }

    /// Refuse a file that lists more than [`MAX_FILE_BLOCKS`] `[[genres]]` and `[[overrides]]`
    /// blocks, before the schema check or any block compiles a glob matcher.
    fn check_block_count(&self, tree: &Node, key: &Key, display: &str) -> Option<Diagnostic> {
        let mut count = 0usize;
        let mut span = None;
        for table in ["genres", "overrides"] {
            if let Some(
                node @ Node {
                    value: Value::Array(items),
                    ..
                },
            ) = tree.get(table)
            {
                count += items.len();
                span.get_or_insert_with(|| node.span.clone());
            }
        }
        if count <= MAX_FILE_BLOCKS {
            return None;
        }
        let refusal = self.over_budget(key, display, ConfigBudget::FileBlocks);
        Some(Diagnostic {
            code: refusal.code,
            message: refusal.message,
            label: ConfigPhrase::LabelTooManyBlocks.text(),
            span: span.unwrap_or(0..0),
            help: refusal.help,
            path: Vec::new(),
            data: refusal.data,
        })
    }

    /// Report a `styles` list that the provider cannot load for a reason that must stop the run.
    fn check_styles(&self, tree: &Node, diagnostics: &mut Vec<Diagnostic>) {
        let Some(node) = tree.get("styles") else {
            return;
        };
        let Value::Array(items) = &node.value else {
            return;
        };
        let listed: Vec<String> = items
            .iter()
            .filter_map(|n| n.as_str().map(str::to_owned))
            .collect();
        if let Err(failure) = self.presets.check_styles(&listed) {
            diagnostics.push(Diagnostic {
                code: failure.code,
                message: failure.message,
                label: ConfigPhrase::LabelListedHere.text(),
                span: node.span.clone(),
                help: None,
                path: vec!["styles".into()],
                data: None,
            });
        }
    }

    fn blocks(
        &self,
        tree: &Node,
        display: &str,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Vec<Block> {
        let mut out = Vec::new();
        for (table, kind) in [
            ("genres", BlockKind::Genre),
            ("overrides", BlockKind::Override),
        ] {
            let Some(Node {
                value: Value::Array(items),
                ..
            }) = tree.get(table)
            else {
                continue;
            };
            let mut seen: HashMap<Vec<String>, usize> = HashMap::new();
            for (index, item) in items.iter().enumerate() {
                let files = strings(item.get("files"));
                let mut scope = files
                    .iter()
                    .map(|f| f.trim().to_owned())
                    .collect::<Vec<_>>();
                scope.sort();
                scope.dedup();
                let files_node = item.get("files");
                let span = files_node.map_or_else(|| item.span.clone(), |n| n.span.clone());
                if let Some(first) = seen.get(&scope) {
                    diagnostics.push(Diagnostic {
                        code: Code::DuplicateScope,
                        message: ConfigText::ScopeRepeats {
                            table: table.to_owned(),
                            index: index.to_string(),
                            first: first.to_string(),
                        }
                        .to_string(),
                        label: ConfigPhrase::LabelSameFiles.text(),
                        span,
                        help: Some(ConfigPhrase::HelpMergeBlocks.text()),
                        path: vec![table.into(), index.to_string(), "files".into()],
                        data: None,
                    });
                    continue;
                }
                seen.insert(scope, index);
                let matcher = match FilesMatcher::new(&files, self.root) {
                    Ok(m) => m,
                    Err(msg) => {
                        diagnostics.push(Diagnostic {
                            code: Code::InvalidValue,
                            message: ConfigText::FilesPatternBad {
                                detail: reason(&msg),
                            }
                            .to_string(),
                            label: ConfigPhrase::LabelBadPattern.text(),
                            span,
                            help: None,
                            path: vec![table.into(), index.to_string(), "files".into()],
                            data: None,
                        });
                        continue;
                    }
                };
                let skip: &[&str] = &["files"];
                out.push(Block {
                    index,
                    kind,
                    order: item.span.start,
                    files,
                    settings: flatten(
                        item,
                        Origin {
                            layer: ConfigLayer::Override,
                            file: display,
                            text,
                        },
                        skip,
                    ),
                    matcher,
                });
            }
        }
        out.sort_by_key(|b| b.order);
        out
    }

    /// The refusal for passing `budget` while loading the `extends` entry of the file `key`
    /// names. The message names only the file, relative to the root, and the limit.
    fn over_budget(&self, key: &Key, display: &str, budget: ConfigBudget) -> Refusal {
        let path = match key {
            Key::Path(p) => slopvac_fs::display_under_root(p, self.root),
            Key::Pack(_) => display.to_owned(),
        };
        let path_text = path.clone();
        let (limit, message, help) = match budget {
            ConfigBudget::ExtendsDepth => {
                let limit = MAX_EXTENDS_DEPTH as u64;
                let message = ConfigText::BudgetDepth {
                    path: path_text,
                    limit,
                };
                (limit, message, ConfigPhrase::HelpFlattenChain)
            }
            ConfigBudget::ExtendsFiles => {
                let limit = MAX_EXTENDS_FILES as u64;
                let message = ConfigText::BudgetFiles {
                    path: path_text,
                    limit,
                };
                (limit, message, ConfigPhrase::HelpRemoveRepeatedFiles)
            }
            ConfigBudget::ExtendsBytes => {
                let limit = MAX_EXTENDS_BYTES;
                let message = ConfigText::BudgetBytes {
                    path: path_text,
                    limit,
                };
                (limit, message, ConfigPhrase::HelpRemoveOrSplit)
            }
            ConfigBudget::FileBlocks => {
                let limit = MAX_FILE_BLOCKS as u64;
                let message = ConfigText::BudgetBlocks {
                    path: path_text,
                    limit,
                };
                (limit, message, ConfigPhrase::HelpMergeOrSplitBlocks)
            }
            ConfigBudget::IndexExtra => {
                let limit = MAX_INDEX_EXTRA as u64;
                let message = ConfigText::BudgetIndexExtra {
                    path: path_text,
                    limit,
                };
                (limit, message, ConfigPhrase::HelpRemoveIndexes)
            }
            ConfigBudget::ExtendsAppliedBytes => {
                let limit = MAX_EXTENDS_APPLIED_BYTES;
                let message = ConfigText::BudgetAppliedBytes {
                    path: path_text,
                    limit,
                };
                (limit, message, ConfigPhrase::HelpRemoveRepeatedLarge)
            }
        };
        Refusal {
            code: Code::BudgetExceeded,
            message: message.to_string(),
            help: Some(help.text()),
            data: Some(Box::new(BudgetData {
                budget,
                limit,
                path,
            })),
        }
    }

    /// Extended files on the stack that have not been applied yet, so each is still to come.
    fn pending(&self) -> usize {
        self.stack.len().saturating_sub(1)
    }

    /// Refuse a file that is not cached when it nests too deep or would pass the file budget.
    fn check_fresh(&self, key: &Key, display: &str) -> Result<(), Refusal> {
        if self.stack.len() > MAX_EXTENDS_DEPTH {
            return Err(self.over_budget(key, display, ConfigBudget::ExtendsDepth));
        }
        if self.files.len() + self.pending() + 1 > MAX_EXTENDS_FILES {
            return Err(self.over_budget(key, display, ConfigBudget::ExtendsFiles));
        }
        Ok(())
    }

    /// Apply the cached expansion of `target`, if there is one. The depth, file and byte
    /// budgets are checked before any application is added. Returns whether it applied.
    fn apply_cached(&mut self, target: &Key, key: &Key, display: &str) -> Result<bool, Refusal> {
        let Some(done) = self.expanded.get(target).cloned() else {
            return Ok(false);
        };
        let deepest = self.stack.len() + done.height - 1;
        if deepest > MAX_EXTENDS_DEPTH {
            return Err(self.over_budget(key, display, ConfigBudget::ExtendsDepth));
        }
        if self.files.len() + self.pending() + done.files.len() > MAX_EXTENDS_FILES {
            return Err(self.over_budget(key, display, ConfigBudget::ExtendsFiles));
        }
        let bytes: u64 = done.files.iter().map(|a| a.bytes).sum();
        if !self.reserve_applied(bytes) {
            return Err(self.over_applied(key, display));
        }
        self.deepest = self.deepest.max(deepest);
        self.files.extend(done.files);
        Ok(true)
    }

    /// Load an extended file that is not cached, and cache its expansion.
    fn load_fresh(&mut self, target: Key, shown: &str, body: String) -> Result<(), ConfigError> {
        let level = self.stack.len();
        let outer = std::mem::replace(&mut self.deepest, level);
        let start = self.files.len();
        self.load_text(&target, shown, body, ConfigLayer::Extends)?;
        let height = self.deepest - level + 1;
        self.deepest = self.deepest.max(outer);
        let files = self.files[start..].to_vec();
        self.expanded.insert(target, Expansion { files, height });
        Ok(())
    }

    /// Count `bytes` more applied text, or return false when that passes the applied-bytes budget.
    fn reserve_applied(&mut self, bytes: u64) -> bool {
        let total = self.applied_bytes.saturating_add(bytes);
        if total > MAX_EXTENDS_APPLIED_BYTES {
            return false;
        }
        self.applied_bytes = total;
        true
    }

    fn over_applied(&self, key: &Key, display: &str) -> Refusal {
        self.over_budget(key, display, ConfigBudget::ExtendsAppliedBytes)
    }

    /// Read the extended file `target`, which the entry `name` names.
    fn read_extended(&self, target: &Key, name: &str) -> Result<String, Refusal> {
        let Key::Path(p) = target else {
            return Ok(String::new());
        };
        read_text(p, MAX_CONFIG_BYTES).map_err(|e| match e {
            ReadFailure::TooLarge(cap) => Refusal::new(
                Code::TooLarge,
                ConfigText::ExtendedTooLarge {
                    name: name.to_owned(),
                    cap,
                }
                .to_string(),
                Some(ConfigPhrase::HelpSplitFile.text()),
            ),
            ReadFailure::Io(e) => Refusal::new(
                Code::ExtendsMissing,
                ConfigText::ExtendedUnreadable {
                    path: safe_display(p, self.root),
                    detail: e.to_string(),
                }
                .to_string(),
                Some(ConfigPhrase::HelpRelativeToEntry.text()),
            ),
        })
    }

    fn extend(
        &mut self,
        tree: &Node,
        key: &Key,
        text: &str,
        display: &str,
    ) -> Result<(), ConfigError> {
        let Some(Node {
            value: Value::Array(entries),
            ..
        }) = tree.get("extends")
        else {
            return Ok(());
        };
        for entry in entries {
            let name = entry.as_str().unwrap_or_default();
            let label = self.error_label(key, display);
            let fail = |r: Refusal| {
                ConfigError::Rejected(Box::new(Rejected {
                    file: label.clone(),
                    path: key_path(key),
                    reads: Vec::new(),
                    text: text.to_owned(),
                    diagnostics: vec![Diagnostic {
                        code: r.code,
                        message: r.message,
                        label: ConfigPhrase::LabelExtendedHere.text(),
                        span: entry.span.clone(),
                        help: r.help,
                        path: vec!["extends".into()],
                        data: r.data,
                    }],
                }))
            };
            let (target, shown, body) = if is_pack_ref(name) {
                let Some(file) = self.presets.pack_config(name) else {
                    return Err(fail(Refusal::new(
                        Code::ExtendsMissing,
                        ConfigText::NoPackConfig {
                            name: name.to_owned(),
                        }
                        .to_string(),
                        None,
                    )));
                };
                (Key::Pack(name.to_owned()), file.display, Some(file.text))
            } else {
                let base = match key {
                    Key::Path(p) => p.parent().map(Path::to_path_buf).unwrap_or_default(),
                    Key::Pack(_) => self.root.to_path_buf(),
                };
                let joined = normalise(&base.join(name));
                let shown = relative_display(&joined, self.root);
                (Key::Path(joined), shown, None)
            };
            if let Some(at) = self.stack.iter().position(|(k, _)| *k == target) {
                let mut chain: Vec<String> = self.stack[at..]
                    .iter()
                    .map(|(k, d)| self.error_label(k, d))
                    .collect();
                chain.push(self.error_label(&target, &shown));
                return Err(fail(Refusal::new(
                    Code::ExtendsCycle,
                    ConfigText::ExtendsLoop {
                        chain: chain.join(" -> "),
                    }
                    .to_string(),
                    Some(ConfigPhrase::HelpBreakLoop.text()),
                )));
            }
            if let Key::Path(p) = &target
                && !self.attempted.contains(p)
            {
                self.attempted.push(p.clone());
            }
            if self.apply_cached(&target, key, display).map_err(fail)? {
                continue;
            }
            self.check_fresh(key, display).map_err(fail)?;
            let body = match body {
                Some(b) => b,
                None => self.read_extended(&target, name).map_err(fail)?,
            };
            self.reads += 1;
            self.bytes += body.len() as u64;
            if self.bytes > MAX_EXTENDS_BYTES {
                return Err(fail(self.over_budget(
                    key,
                    display,
                    ConfigBudget::ExtendsBytes,
                )));
            }
            // Reserve the file's bytes before it loads, so its own `extends` entries see them.
            if !self.reserve_applied(body.len() as u64) {
                return Err(fail(self.over_applied(key, display)));
            }
            self.load_fresh(target, &shown, body)?;
        }
        Ok(())
    }
}

/// A path for a message: relative to the project root when it sits under it, else
/// `<external>/<file name>`. Unlike [`relative_display`] it never shows the directories
/// above or beside the root.
fn safe_display(path: &Path, root: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(root) {
        return rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
    }
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    format!("<external>/{name}")
}

fn is_pack_ref(name: &str) -> bool {
    name.contains(':') && !Path::new(name).is_absolute()
}

fn strings(node: Option<&Node>) -> Vec<String> {
    match node.map(|n| &n.value) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn reason(err: &FilesError) -> String {
    match err {
        FilesError::Pattern { source, .. } | FilesError::Set(source) => source.to_string(),
    }
}

fn compile(patterns: &[String], root: &Path) -> Result<Gitignore, String> {
    let mut builder = GitignoreBuilder::new(root);
    for p in patterns {
        builder.add_line(None, p).map_err(|e| e.to_string())?;
    }
    builder.build().map_err(|e| e.to_string())
}

fn check_exclude(tree: &Node, diagnostics: &mut Vec<Diagnostic>) {
    let Some(Node {
        value: Value::Array(items),
        ..
    }) = tree.get("exclude")
    else {
        return;
    };
    for (i, item) in items.iter().enumerate() {
        let Some(pattern) = item.as_str() else {
            continue;
        };
        if let Err(msg) = compile(&[pattern.to_owned()], Path::new("/")) {
            diagnostics.push(Diagnostic {
                code: Code::InvalidValue,
                message: ConfigText::PatternBad { detail: msg }.to_string(),
                label: ConfigPhrase::LabelBadPattern.text(),
                span: item.span.clone(),
                help: None,
                path: vec!["exclude".into(), i.to_string()],
                data: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::{PackFile, Presets};
    use crate::source::Setting;

    /// No packs behind the loader.
    struct NoPresets;

    impl Presets for NoPresets {
        fn style(&self, _name: &str) -> Option<Vec<Setting>> {
            None
        }

        fn genre_chain(&self, _name: &str) -> Vec<Vec<Setting>> {
            Vec::new()
        }

        fn pack_config(&self, _name: &str) -> Option<PackFile> {
            None
        }
    }
    use rstest::rstest;

    #[test]
    fn a_file_extended_many_times_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        for i in 0..6 {
            let next = format!("d{}.toml", i + 1);
            fs::write(
                root.join(format!("d{i}.toml")),
                format!("schema_version = 3\nextends = [\"{next}\", \"{next}\"]\n"),
            )
            .unwrap();
        }
        fs::write(root.join("d6.toml"), "schema_version = 3\n").unwrap();
        let path = root.join("d0.toml");
        let (config, reads) = load_counted(&path, "d0.toml", &NoPresets).unwrap();
        // 7 files in the graph, 126 applications below the root.
        assert_eq!(reads, 7);
        assert_eq!(config.files.len(), 127);
    }

    #[test]
    fn config_text_loads_as_the_file_at_a_path_that_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slopvac.toml");
        let config = load_text(
            &path,
            "slopvac.toml",
            "schema_version = 3\nexclude = [\"drafts/\"]\n".to_owned(),
            &NoPresets,
        )
        .unwrap();
        assert_eq!(config.root, fs::canonicalize(dir.path()).unwrap());
        assert!(config.is_excluded(Path::new("drafts/a.md")));
        assert!(!config.is_excluded(Path::new("a.md")));
        assert!(!path.exists());
    }

    #[test]
    fn a_file_sharing_no_prefix_with_the_root_shows_as_external() {
        // Absolute paths on Unix always share `/`, so relative paths stand in for another drive.
        assert_eq!(
            relative_display(Path::new("other/base.toml"), Path::new("proj/dir")),
            "<external>/base.toml"
        );
    }

    #[test]
    fn a_file_sharing_a_prefix_shows_relative_with_parent_segments() {
        assert_eq!(
            relative_display(Path::new("a/shared/base.toml"), Path::new("a/proj")),
            "../shared/base.toml"
        );
    }

    // `Path` parses these prefixes on Windows without touching the filesystem, so no real
    // drive or share is needed.
    #[cfg(windows)]
    #[test]
    fn a_plain_and_a_verbatim_drive_are_one_place() {
        assert_eq!(
            relative_display(
                Path::new(r"C:\w\shared\b.toml"),
                Path::new(r"\\?\c:\w\proj")
            ),
            "../shared/b.toml"
        );
    }

    #[cfg(windows)]
    #[test]
    fn another_drive_shows_as_external() {
        assert_eq!(
            relative_display(Path::new(r"D:\w\b.toml"), Path::new(r"\\?\C:\w\proj")),
            "<external>/b.toml"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_plain_and_a_verbatim_unc_share_are_one_place_ignoring_case() {
        assert_eq!(
            relative_display(
                Path::new(r"\\SERVER\Share\w\shared\b.toml"),
                Path::new(r"\\?\UNC\server\share\w\proj")
            ),
            "../shared/b.toml"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_different_unc_share_shows_as_external() {
        assert_eq!(
            relative_display(
                Path::new(r"\\server\other\w\b.toml"),
                Path::new(r"\\?\UNC\server\share\w\proj")
            ),
            "<external>/b.toml"
        );
    }

    #[rstest]
    #[case::parent("../README.md")]
    #[case::deep_parent("../../elsewhere/README.md")]
    #[case::absolute("/elsewhere/README.md")]
    #[case::escapes_mid_path("a/../../README.md")]
    fn a_path_outside_the_root_is_not_excluded(#[case] rel: &str) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slopvac.toml");
        fs::write(&path, "schema_version = 3\nexclude = [\"*.md\"]\n").unwrap();
        let config = load(&path, &NoPresets).unwrap();
        assert!(config.is_excluded(Path::new("README.md")));
        assert!(!config.is_excluded(Path::new(rel)));
    }

    #[test]
    fn a_path_that_climbs_and_returns_inside_the_root_is_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slopvac.toml");
        fs::write(&path, "schema_version = 3\nexclude = [\"*.md\"]\n").unwrap();
        let config = load(&path, &NoPresets).unwrap();
        assert!(config.is_excluded(Path::new("a/../README.md")));
    }

    #[rstest]
    #[case::inside("README.md", true)]
    #[case::parent("../README.md", false)]
    #[case::absolute("/elsewhere/README.md", false)]
    #[case::escapes_mid_path("a/../../README.md", false)]
    #[case::stays_inside("a/../README.md", true)]
    fn a_block_applies_only_to_a_path_under_the_root(#[case] rel: &str, #[case] applies: bool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slopvac.toml");
        fs::write(
            &path,
            "schema_version = 3\n[[genres]]\nfiles = [\"*.md\"]\ngenre = \"readme\"\n",
        )
        .unwrap();
        let config = load(&path, &NoPresets).unwrap();
        let block = &config.files.last().unwrap().blocks[0];
        assert_eq!(block.matches(Path::new(rel)), applies);
    }
}
