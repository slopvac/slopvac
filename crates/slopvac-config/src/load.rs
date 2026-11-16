//! Read a `slopvac.toml` and the configs it extends into an ordered list of files.

use std::fs;
use std::path::{Component, Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::Value as Json;

use crate::diag::{Code, Diagnostic};
use crate::error::{ConfigError, Rejected};
use crate::flatten::{Origin, flatten};
use crate::presets::Presets;
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
    let text = fs::read_to_string(&abs).map_err(|source| ConfigError::io(label, source))?;
    let mut loader = Loader {
        root: &root,
        presets,
        stack: Vec::new(),
        files: Vec::new(),
    };
    loader.load_text(
        &Key::Path(abs.clone()),
        &relative_display(&abs, &root),
        text,
        Layer::Project,
    )?;
    let exclude = build_exclude(&loader.files, &root);
    let files = loader.files;
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

/// The path of `file` relative to `root`, with `..` for each directory it climbs out of.
fn relative_display(file: &Path, root: &Path) -> String {
    let file = normalise(file);
    let f: Vec<_> = file.components().collect();
    let r: Vec<_> = root.components().collect();
    let common = f.iter().zip(&r).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return file.display().to_string();
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    Path(PathBuf),
    Pack(String),
}

struct Loader<'a> {
    root: &'a Path,
    presets: &'a dyn Presets,
    stack: Vec<(Key, String)>,
    files: Vec<ConfigFile>,
}

impl Loader<'_> {
    fn load_text(
        &mut self,
        key: &Key,
        display: &str,
        text: String,
        layer: Layer,
    ) -> Result<(), ConfigError> {
        let reject = |text: String, diagnostics: Vec<Diagnostic>| {
            ConfigError::Rejected(Box::new(Rejected {
                file: display.to_owned(),
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
            let fail = |code: Code, message: String, help: Option<String>| {
                ConfigError::Rejected(Box::new(Rejected {
                    file: display.to_owned(),
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
                let mut chain: Vec<String> =
                    self.stack[at..].iter().map(|(_, d)| d.clone()).collect();
                chain.push(shown);
                return Err(fail(
                    Code::ExtendsCycle,
                    format!("`extends` loops: {}", chain.join(" -> ")),
                    Some("remove one entry from the loop".into()),
                ));
            }
            let body = match body {
                Some(b) => b,
                None => match &target {
                    Key::Path(p) => fs::read_to_string(p).map_err(|e| {
                        fail(
                            Code::ExtendsMissing,
                            format!("cannot read the extended config `{name}`: {e}"),
                            Some("paths are relative to the file that holds the entry".into()),
                        )
                    })?,
                    Key::Pack(_) => String::new(),
                },
            };
            self.load_text(&target, &shown, body, Layer::Extends)?;
        }
        Ok(())
    }
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
