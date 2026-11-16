//! Read a `slopvac.toml` and the configs it extends into an ordered list of files.

use std::fs;
use std::path::{Component, Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::Value as Json;

use crate::diag::{Code, Diagnostic};
use crate::error::{ConfigError, Rejected};
use crate::flatten::{Origin, flatten_with};
use crate::presets::Presets;
use crate::schema::validate_config;
use crate::source::{Layer, Setting};
use crate::tree::{LineIndex, Node, Value, parse_toml};
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
    let abs = fs::canonicalize(path).map_err(|source| ConfigError::io(label, source))?;
    let root = abs.parent().map_or_else(PathBuf::new, Path::to_path_buf);
    let text = crate::read::read_text(&abs).map_err(|source| ConfigError::io(label, source))?;
    let mut loader = Loader {
        root: &root,
        presets,
        stack: Vec::new(),
        table: Vec::new(),
        order: Ordered::default(),
        entries: 0,
        ids: std::collections::HashMap::new(),
        closure: Vec::new(),
        // The project file counts against the byte budget like every extended file.
        bytes: text.len(),
    };
    loader.load_text(
        &Key::Path(abs.clone()),
        &relative_display(&abs, &root),
        text,
        Layer::Project,
    )?;
    // The order is the expansion of `extends` with each file at its last position, so a later
    // entry wins even when an earlier entry also reaches the same file.
    let files: Vec<ConfigFile> = loader
        .order
        .ids()
        .into_iter()
        .map(|i| loader.table[i].clone())
        .collect();
    let exclude = build_exclude(&files, &root);
    Ok(LoadedConfig {
        root,
        files,
        exclude,
    })
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

/// Longest `extends` chain.
const MAX_EXTENDS_DEPTH: usize = 16;
/// Most distinct files in one stack.
const MAX_EXTENDS_FILES: usize = 64;
/// Most bytes in one stack.
const MAX_EXTENDS_BYTES: usize = 4 * 1024 * 1024;
/// Most `extends` entries in one stack, repeated references included. A file reached again
/// replays its closure, so the entries bound the work.
const MAX_EXTENDS_ENTRIES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Path(PathBuf),
    Pack(String),
}

struct Loader<'a> {
    root: &'a Path,
    presets: &'a dyn Presets,
    stack: Vec<(Key, String)>,
    /// Each distinct file, parsed once, in the order it was first loaded.
    table: Vec<ConfigFile>,
    /// Indexes into `table` in application order, each file at its last position.
    order: Ordered,
    /// The table index of every finished file, by canonical path.
    ids: std::collections::HashMap<Key, usize>,
    /// For each table entry: it and everything it extends, in application order.
    closure: Vec<Vec<usize>>,
    /// `extends` entries read so far, in every file, repeats included.
    entries: usize,
    bytes: usize,
}

/// File ids in application order where a file that appears again moves to the end. A move is
/// one map lookup and one tombstone, so replaying a repeated reference costs the size of its
/// closure and no scan.
#[derive(Debug, Default)]
struct Ordered {
    slots: Vec<Option<usize>>,
    at: std::collections::HashMap<usize, usize>,
}

impl Ordered {
    fn push(&mut self, id: usize) {
        if let Some(old) = self.at.insert(id, self.slots.len()) {
            self.slots[old] = None;
        }
        self.slots.push(Some(id));
    }

    fn ids(&self) -> Vec<usize> {
        self.slots.iter().flatten().copied().collect()
    }
}

/// Largest number of `[[overrides]]` plus `[[genres]]` blocks in one file.
const MAX_BLOCKS_PER_FILE: usize = 1000;
/// Most patterns in one block's `files`.
const MAX_GLOBS_PER_BLOCK: usize = 100;

impl Loader<'_> {
    /// Whether loading one more file would break the depth or file budget.
    ///
    /// Every distinct file counts: the finished ones, the ancestors still loading, and the new one.
    fn budget_exceeded(&self) -> Option<(String, &'static str)> {
        if self.stack.len() >= MAX_EXTENDS_DEPTH {
            return Some((
                format!("`extends` is nested more than {MAX_EXTENDS_DEPTH} files deep"),
                "flatten the chain",
            ));
        }
        if self.ids.len() + self.stack.len() + 1 > MAX_EXTENDS_FILES {
            return Some((
                format!("`extends` reaches more than {MAX_EXTENDS_FILES} files"),
                "extend fewer files",
            ));
        }
        None
    }

    /// Count `added` more `extends` entries; the rejection when the stack passes its budget.
    fn entries_exceeded(
        &mut self,
        tree: &Node,
        added: usize,
        (display, layer): (&str, Layer),
        text: &str,
    ) -> Option<ConfigError> {
        self.entries += added;
        if self.entries <= MAX_EXTENDS_ENTRIES {
            return None;
        }
        let span = tree.get("extends").map_or(0..0, |n| n.span.clone());
        Some(ConfigError::Rejected(Box::new(Rejected {
            file: display.to_owned(),
            layer: Some(layer),
            root: Some(self.root.to_path_buf()),
            text: text.to_owned(),
            diagnostics: vec![Diagnostic {
                code: Code::ExtendsLimit,
                message: format!(
                    "`extends` lists more than {MAX_EXTENDS_ENTRIES} entries in total"
                ),
                label: "too many entries".into(),
                span,
                help: Some("list each file once".into()),
                path: vec!["extends".into()],
            }],
        })))
    }

    /// The text of an extended file (read from disk unless a pack supplied it), counted against
    /// the byte budget. The error is the diagnostic code, message and help.
    fn take_body(
        &mut self,
        target: &Key,
        body: Option<String>,
    ) -> Result<String, (Code, String, Option<String>)> {
        let body = match body {
            Some(b) => b,
            None => match target {
                Key::Path(p) => crate::read::read_text(p).map_err(|e| {
                    (
                        Code::ExtendsMissing,
                        format!(
                            "cannot read the extended config `{}`: {}",
                            safe_display(p, self.root),
                            crate::error::reason(&e)
                        ),
                        Some("paths are relative to the file that holds the entry".into()),
                    )
                })?,
                Key::Pack(_) => String::new(),
            },
        };
        self.bytes += body.len();
        if self.bytes > MAX_EXTENDS_BYTES {
            return Err((
                Code::ExtendsLimit,
                format!("`extends` reads more than {MAX_EXTENDS_BYTES} bytes in total"),
                Some("extend smaller files".into()),
            ));
        }
        Ok(body)
    }

    /// Move `id` to the end of the application order.
    fn touch(&mut self, id: usize) {
        self.order.push(id);
    }

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
                layer: Some(layer),
                root: Some(self.root.to_path_buf()),
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
        let block_count = ["overrides", "genres"]
            .iter()
            .filter_map(|t| match tree.get(t).map(|n| &n.value) {
                Some(Value::Array(items)) => Some(items.len()),
                _ => None,
            })
            .sum::<usize>();
        if block_count > MAX_BLOCKS_PER_FILE {
            let d = Diagnostic {
                code: Code::ExtendsLimit,
                message: format!(
                    "the file has {block_count} `overrides` and `genres` blocks; the limit is {MAX_BLOCKS_PER_FILE}"
                ),
                label: "too many blocks".into(),
                span: 0..0,
                help: Some("merge blocks that share files".into()),
                path: Vec::new(),
            };
            return Err(reject(text, vec![d]));
        }
        let mut diagnostics = validate_config(&tree, &text);
        if diagnostics.is_empty() {
            check_exclude(&tree, &mut diagnostics);
        }
        let lines = LineIndex::new(&text);
        let blocks = if diagnostics.is_empty() {
            self.blocks(&tree, display, &text, &lines, &mut diagnostics)
        } else {
            Vec::new()
        };
        if !diagnostics.is_empty() {
            return Err(reject(text, diagnostics));
        }
        self.stack.push((key.clone(), display.to_owned()));
        let extended = self.extend(&tree, key, &text, display, layer);
        self.stack.pop();
        let mut closure = extended?;
        let settings = flatten_with(
            &tree,
            Origin {
                layer,
                file: display,
                text: &text,
            },
            &["schema_version", "extends", "overrides", "genres"],
            &lines,
        );
        let id = self.table.len();
        self.table.push(ConfigFile {
            display: display.to_owned(),
            layer,
            settings,
            blocks,
        });
        self.ids.insert(key.clone(), id);
        self.touch(id);
        closure.push(id);
        self.closure.push(closure);
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

    fn blocks(
        &self,
        tree: &Node,
        display: &str,
        text: &str,
        lines: &LineIndex<'_>,
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
            let mut seen: std::collections::HashMap<Vec<String>, usize> =
                std::collections::HashMap::new();
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
                if files.len() > MAX_GLOBS_PER_BLOCK {
                    diagnostics.push(Diagnostic {
                        code: Code::ExtendsLimit,
                        message: format!(
                            "`{table}[{index}]` lists {} patterns; the limit is {MAX_GLOBS_PER_BLOCK}",
                            files.len()
                        ),
                        label: "too many patterns".into(),
                        span,
                        help: None,
                        path: vec![table.into(), index.to_string(), "files".into()],
                    });
                    continue;
                }
                if let Some(first) = seen.get(&scope) {
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
                seen.insert(scope, index);
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
                    settings: flatten_with(
                        item,
                        Origin {
                            layer: Layer::Override,
                            file: display,
                            text,
                        },
                        skip,
                        lines,
                    ),
                    matcher,
                });
            }
        }
        out.sort_by_key(|b| b.order);
        out
    }

    fn extend(
        &mut self,
        tree: &Node,
        key: &Key,
        text: &str,
        display: &str,
        layer: Layer,
    ) -> Result<Vec<usize>, ConfigError> {
        let Some(Node {
            value: Value::Array(entries),
            ..
        }) = tree.get("extends")
        else {
            return Ok(Vec::new());
        };
        if let Some(rejection) = self.entries_exceeded(tree, entries.len(), (display, layer), text)
        {
            return Err(rejection);
        }
        let mut acc = Ordered::default();
        let root = self.root;
        for entry in entries {
            let name = entry.as_str().unwrap_or_default();
            let label = self.error_label(key, display);
            let fail = |code: Code, message: String, help: Option<String>| {
                ConfigError::Rejected(Box::new(Rejected {
                    file: label.clone(),
                    layer: Some(layer),
                    root: Some(root.to_path_buf()),
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
            // The canonical path is the identity, so a link or a different spelling of a file that
            // is already counted is the same file and costs no extra slot.
            let target = match target {
                Key::Path(p) => Key::Path(fs::canonicalize(&p).unwrap_or(p)),
                pack @ Key::Pack(_) => pack,
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
            if let Some(&id) = self.ids.get(&target) {
                // Already loaded: no new read, but the file and what it extends apply again here,
                // so this later entry wins over an earlier one that reached it first.
                for i in self.closure[id].clone() {
                    self.touch(i);
                    acc.push(i);
                }
                continue;
            }
            if let Some((message, help)) = self.budget_exceeded() {
                return Err(fail(Code::ExtendsLimit, message, Some(help.into())));
            }
            let body = self
                .take_body(&target, body)
                .map_err(|(code, message, help)| fail(code, message, help))?;
            self.load_text(&target, &shown, body, Layer::Extends)?;
            let id = self.ids[&target];
            for i in self.closure[id].clone() {
                acc.push(i);
            }
        }
        Ok(acc.ids())
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
