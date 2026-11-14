//! Find the nearest `slopvac.toml` for a directory, once per directory.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::diag::{Code, Diagnostic};
use crate::error::{ConfigError, Rejected};
use crate::load::{LoadedConfig, load};
use crate::presets::Presets;
use crate::tree::{Node, parse_toml};

/// The config file name. 3.0 reads no other.
pub const FILE_NAME: &str = "slopvac.toml";

/// Finds and loads configs, and remembers the answer for each directory.
pub struct Discovery<'p> {
    presets: &'p dyn Presets,
    found: Mutex<HashMap<PathBuf, Option<PathBuf>>>,
    loaded: Mutex<HashMap<PathBuf, Arc<LoadedConfig>>>,
}

impl<'p> Discovery<'p> {
    /// A discovery that reads pack-supplied layers from `presets`.
    #[must_use]
    pub fn new(presets: &'p dyn Presets) -> Self {
        Self {
            presets,
            found: Mutex::default(),
            loaded: Mutex::default(),
        }
    }

    /// The config that applies to files in `dir`, or `None` when no config is on the path.
    ///
    /// The walk starts in `dir` and goes up one directory at a time. It checks a directory for
    /// `slopvac.toml`, then ends if that directory holds `.git`. A 2.x config met on the way is
    /// an error.
    pub fn nearest(&self, dir: &Path) -> Result<Option<Arc<LoadedConfig>>, ConfigError> {
        let dir = fs::canonicalize(dir)
            .map_err(|source| ConfigError::io(dir.display().to_string(), source))?;
        let Some(path) = self.find(&dir)? else {
            return Ok(None);
        };
        self.load_cached(path).map(Some)
    }

    /// Load the config at `path` and apply it to every file, as `--config` does.
    pub fn explicit(&self, path: &Path) -> Result<Arc<LoadedConfig>, ConfigError> {
        let abs = fs::canonicalize(path)
            .map_err(|source| ConfigError::io(path.display().to_string(), source))?;
        self.load_cached(abs)
    }

    fn load_cached(&self, path: PathBuf) -> Result<Arc<LoadedConfig>, ConfigError> {
        if let Some(hit) = lock(&self.loaded).get(&path) {
            return Ok(Arc::clone(hit));
        }
        let config = Arc::new(load(&path, self.presets)?);
        lock(&self.loaded).insert(path, Arc::clone(&config));
        Ok(config)
    }

    fn find(&self, start: &Path) -> Result<Option<PathBuf>, ConfigError> {
        let mut visited = Vec::new();
        let mut cur = Some(start);
        let mut result = None;
        while let Some(dir) = cur {
            if let Some(hit) = lock(&self.found).get(dir) {
                result.clone_from(hit);
                break;
            }
            visited.push(dir.to_path_buf());
            let candidate = dir.join(FILE_NAME);
            if candidate.is_file() {
                result = Some(candidate);
                break;
            }
            reject_legacy(dir)?;
            if dir.join(".git").exists() {
                break;
            }
            cur = dir.parent();
        }
        let mut found = lock(&self.found);
        for v in visited {
            found.insert(v, result.clone());
        }
        Ok(result)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A `.slopvac.toml` file or a `[tool.slopvac]` table in `dir` is a 2.x config.
fn reject_legacy(dir: &Path) -> Result<(), ConfigError> {
    let dotfile = dir.join(".slopvac.toml");
    if dotfile.is_file() {
        let text = read(&dotfile)?;
        return Err(legacy(
            ".slopvac.toml",
            text.clone(),
            0..text.find('\n').unwrap_or(text.len()),
            "`.slopvac.toml` is the 2.x config file name",
        ));
    }
    let pyproject = dir.join("pyproject.toml");
    if pyproject.is_file() {
        let text = read(&pyproject)?;
        if let Ok(root) = parse_toml(&text)
            && let Some(span) = tool_slopvac_span(&root)
        {
            return Err(legacy(
                "pyproject.toml",
                text,
                span,
                "a `[tool.slopvac]` table is a 2.x config",
            ));
        }
    }
    Ok(())
}

fn tool_slopvac_span(root: &Node) -> Option<std::ops::Range<usize>> {
    root.get("tool")?
        .entry("slopvac")
        .map(|e| e.key_span.clone())
}

fn read(path: &Path) -> Result<String, ConfigError> {
    let label = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    fs::read_to_string(path).map_err(|source| ConfigError::io(label, source))
}

fn legacy(file: &str, text: String, span: std::ops::Range<usize>, message: &str) -> ConfigError {
    ConfigError::Rejected(Box::new(Rejected {
        file: file.to_owned(),
        text,
        diagnostics: vec![Diagnostic {
            code: Code::LegacyVersion,
            message: message.to_owned(),
            label: "slopvac 3 reads only `slopvac.toml`".into(),
            span,
            help: Some("run `slopvac migrate` to convert it".into()),
            path: Vec::new(),
        }],
    }))
}
