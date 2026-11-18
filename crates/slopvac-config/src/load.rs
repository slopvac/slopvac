//! Read a `slopvac.toml` and the configs it extends into an ordered list of files.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::Value as Json;

use crate::diag::{Code, Diagnostic};
use crate::error::{ConfigError, Rejected};
use crate::flatten::{Origin, flatten};
use crate::presets::Presets;
use crate::read::{
    MAX_CONFIG_BYTES, MAX_EXTENDS_BYTES, MAX_EXTENDS_DEPTH, MAX_EXTENDS_FILES, ReadFailure,
    read_text,
};
use crate::schema::validate_config;
use crate::source::{Layer, Setting};
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
    matcher: Gitignore,
}

impl Block {
    /// Whether the block applies to `rel`, a path relative to the project root.
    #[must_use]
    pub fn matches(&self, rel: &Path) -> bool {
        !rel.starts_with("..")
            && self
                .matcher
                .matched_path_or_any_parents(rel, false)
                .is_ignore()
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
    pub layer: Layer,
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
    pub files: Vec<ConfigFile>,
    exclude: Gitignore,
}

impl LoadedConfig {
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

    /// A config with no files, for a project that has none: only the core defaults apply and
    /// nothing is excluded. `root` is the directory that stands in for the project root.
    #[must_use]
    pub fn without_files(root: PathBuf) -> Self {
        Self {
            root,
            files: Vec::new(),
            exclude: Gitignore::empty(),
        }
    }

    /// Whether `rel`, a path relative to the project root, is outside the lint run.
    #[must_use]
    pub fn is_excluded(&self, rel: &Path) -> bool {
        self.exclude
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

/// As [`load_labeled`], also returning how many files were read.
fn load_counted(
    path: &Path,
    label: &str,
    presets: &dyn Presets,
) -> Result<(LoadedConfig, usize), ConfigError> {
    let abs = fs::canonicalize(path).map_err(|source| ConfigError::io(label, source))?;
    let root = abs.parent().map_or_else(PathBuf::new, Path::to_path_buf);
    let text = ConfigError::read_config(&abs, label)?;
    let mut loader = Loader {
        root: &root,
        presets,
        stack: Vec::new(),
        files: Vec::new(),
        expanded: HashMap::new(),
        bytes: text.len() as u64,
        reads: 1,
    };
    loader.load_text(
        &Key::Path(abs.clone()),
        &relative_display(&abs, &root),
        text,
        Layer::Project,
    )?;
    let exclude = build_exclude(&loader.files, &root);
    let reads = loader.reads;
    let files = loader.files;
    Ok((
        LoadedConfig {
            root,
            files,
            exclude,
        },
        reads,
    ))
}

fn build_exclude(files: &[ConfigFile], root: &Path) -> Gitignore {
    let patterns = files
        .iter()
        .flat_map(|f| f.settings.iter())
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

/// Lexically normalise `path`: drop `.` and fold `..` against the preceding component.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
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

/// A diagnostic code, message and help for an `extends` entry that is refused.
type Refusal = (Code, String, Option<String>);

struct Loader<'a> {
    root: &'a Path,
    presets: &'a dyn Presets,
    stack: Vec<(Key, String)>,
    files: Vec<ConfigFile>,
    /// For each extended file already loaded, the files it expanded to, in apply order. A file
    /// that several others extend is read and parsed once and its expansion is copied after that.
    expanded: HashMap<Key, Vec<ConfigFile>>,
    /// Bytes read so far from distinct reads.
    bytes: u64,
    /// Files read so far.
    reads: usize,
}

impl Loader<'_> {
    fn load_text(
        &mut self,
        key: &Key,
        display: &str,
        text: String,
        layer: Layer,
    ) -> Result<(), ConfigError> {
        let label = self.error_label(key, display);
        let reject = |text: String, diagnostics: Vec<Diagnostic>| {
            ConfigError::Rejected(Box::new(Rejected {
                file: label.clone(),
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
                    message: format!("not valid TOML: {}", e.message),
                    label: "the parser stopped here".into(),
                    span,
                    help: None,
                    path: Vec::new(),
                };
                return Err(reject(text, vec![d]));
            }
        };
        if let Err(d) = check_version(&tree, &text) {
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
        self.files.push(ConfigFile {
            display: display.to_owned(),
            layer,
            settings,
            blocks,
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
                label: "listed here".into(),
                span: node.span.clone(),
                help: None,
                path: vec!["styles".into()],
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
            let mut seen: Vec<(Vec<String>, usize)> = Vec::new();
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
                if let Some((_, first)) = seen.iter().find(|(s, _)| *s == scope) {
                    diagnostics.push(Diagnostic {
                        code: Code::DuplicateScope,
                        message: format!(
                            "`{table}[{index}]` repeats the `files` of `{table}[{first}]`"
                        ),
                        label: "same files as an earlier block".into(),
                        span,
                        help: Some("merge the two blocks".into()),
                        path: vec![table.into(), index.to_string(), "files".into()],
                    });
                    continue;
                }
                seen.push((scope, index));
                let matcher = match compile(&files, self.root) {
                    Ok(m) => m,
                    Err(msg) => {
                        diagnostics.push(Diagnostic {
                            code: Code::InvalidValue,
                            message: format!("a `files` pattern does not compile: {msg}"),
                            label: "bad pattern".into(),
                            span,
                            help: None,
                            path: vec![table.into(), index.to_string(), "files".into()],
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
                            layer: Layer::Override,
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

    /// The refusal for loading `more` files when that passes the file budget.
    fn files_over_budget(&self, more: usize) -> Option<Refusal> {
        (self.files.len() + more > MAX_EXTENDS_FILES).then(|| {
            (
                Code::BudgetExceeded,
                format!("the `extends` graph expands to more than {MAX_EXTENDS_FILES} files"),
                Some("remove entries that extend the same files again".into()),
            )
        })
    }

    /// The refusal for one more level of nesting when that passes the depth limit.
    fn too_deep(&self) -> Option<Refusal> {
        (self.stack.len() > MAX_EXTENDS_DEPTH).then(|| {
            (
                Code::ExtendsTooDeep,
                format!("`extends` nests deeper than {MAX_EXTENDS_DEPTH} levels"),
                Some("flatten the chain".into()),
            )
        })
    }

    /// Read the extended file `target`, which the entry `name` names.
    fn read_extended(&self, target: &Key, name: &str) -> Result<String, Refusal> {
        let Key::Path(p) = target else {
            return Ok(String::new());
        };
        read_text(p, MAX_CONFIG_BYTES).map_err(|e| match e {
            ReadFailure::TooLarge(cap) => (
                Code::TooLarge,
                format!("the extended config `{name}` is larger than {cap} bytes"),
                Some("split the file or remove unused settings".into()),
            ),
            ReadFailure::Io(e) => (
                Code::ExtendsMissing,
                format!(
                    "cannot read the extended config `{}`: {e}",
                    safe_display(p, self.root)
                ),
                Some("paths are relative to the file that holds the entry".into()),
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
            let fail = |code: Code, message: String, help: Option<String>| {
                ConfigError::Rejected(Box::new(Rejected {
                    file: label.clone(),
                    text: text.to_owned(),
                    diagnostics: vec![Diagnostic {
                        code,
                        message,
                        label: "extended here".into(),
                        span: entry.span.clone(),
                        help,
                        path: vec!["extends".into()],
                    }],
                }))
            };
            let (target, shown, body) = if is_pack_ref(name) {
                let Some(file) = self.presets.pack_config(name) else {
                    return Err(fail(
                        Code::ExtendsMissing,
                        format!("no active pack provides the config `{name}`"),
                        None,
                    ));
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
                return Err(fail(
                    Code::ExtendsCycle,
                    format!("`extends` loops: {}", chain.join(" -> ")),
                    Some("remove one entry from the loop".into()),
                ));
            }
            if let Some(done) = self.expanded.get(&target) {
                if let Some((code, message, help)) = self.files_over_budget(done.len()) {
                    return Err(fail(code, message, help));
                }
                let copy = done.clone();
                self.files.extend(copy);
                continue;
            }
            if let Some((code, message, help)) = self.too_deep() {
                return Err(fail(code, message, help));
            }
            if let Some((code, message, help)) = self.files_over_budget(1) {
                return Err(fail(code, message, help));
            }
            let body = match body {
                Some(b) => b,
                None => self
                    .read_extended(&target, name)
                    .map_err(|(code, message, help)| fail(code, message, help))?,
            };
            self.reads += 1;
            self.bytes += body.len() as u64;
            if self.bytes > MAX_EXTENDS_BYTES {
                return Err(fail(
                    Code::BudgetExceeded,
                    format!("the `extends` graph reads more than {MAX_EXTENDS_BYTES} bytes"),
                    Some("remove entries or split the files".into()),
                ));
            }
            let start = self.files.len();
            self.load_text(&target, &shown, body, Layer::Extends)?;
            self.expanded.insert(target, self.files[start..].to_vec());
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
                message: format!("the pattern does not compile: {msg}"),
                label: "bad pattern".into(),
                span: item.span.clone(),
                help: None,
                path: vec!["exclude".into(), i.to_string()],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::NoPresets;

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
}
