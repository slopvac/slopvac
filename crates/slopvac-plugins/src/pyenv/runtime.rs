//! The `[plugin]` fields of the pack manifest that select a managed Python
//! environment (`runtime`, `python`, `command`, `args`).
//!
//! This reads only what the environment needs. The pack loader owns full
//! manifest validation; the manifest bytes are part of the trust hash, so an
//! edit here changes nothing without a new trust decision.

use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{ManagedEnv, PyEnvError};

/// File name of the manifest at the root of a pack.
pub const MANIFEST_FILE: &str = "slopvac-pack.toml";

const MAX_MANIFEST_BYTES: u64 = 1 << 20;

#[derive(Debug, Deserialize)]
struct Manifest {
    kind: Option<String>,
    plugin: Option<PluginTable>,
}

#[derive(Debug, Default, Deserialize)]
struct PluginTable {
    runtime: Option<String>,
    python: Option<String>,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
}

/// A plugin that runs in a managed Python environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonRuntime {
    /// The interpreter request, for example `3.12`.
    pub python: String,
    /// The script to run, relative to the plugin tree.
    pub command: String,
    /// Arguments after the script.
    pub args: Vec<String>,
}

impl PythonRuntime {
    /// The runtime the manifest at the root of `root` selects, or `None` for a
    /// tree without a manifest, a data pack and a native plugin.
    ///
    /// # Errors
    /// [`PyEnvError::Manifest`] for `runtime = "python"` without a valid
    /// `python` request or `command`, for `python` without that runtime, for
    /// an unknown runtime, and for a manifest that is too large or not TOML.
    pub fn read(root: &Path) -> Result<Option<Self>, PyEnvError> {
        let path = root.join(MANIFEST_FILE);
        let bad = |why: String| PyEnvError::Manifest {
            path: path.clone(),
            why,
        };
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(bad(e.to_string())),
        };
        if !meta.is_file() || meta.len() > MAX_MANIFEST_BYTES {
            return Err(bad("it is not a regular file of at most 1 MiB".into()));
        }
        let mut text = String::new();
        File::open(&path)
            .and_then(|f| f.take(MAX_MANIFEST_BYTES).read_to_string(&mut text))
            .map_err(|e| bad(e.to_string()))?;
        let manifest: Manifest = toml::from_str(&text).map_err(|e| bad(e.message().to_owned()))?;
        let Some(plugin) = manifest.plugin else {
            return Ok(None);
        };
        match plugin.runtime.as_deref() {
            None | Some("native") => {
                if plugin.python.is_some() {
                    return Err(bad("`python` needs `runtime = \"python\"`".into()));
                }
                Ok(None)
            }
            Some("python") => {
                if manifest.kind.as_deref() != Some("plugin") {
                    return Err(bad("`runtime` belongs to a `kind = \"plugin\"` pack".into()));
                }
                let python = plugin
                    .python
                    .ok_or_else(|| bad("`runtime = \"python\"` needs `python`".into()))?;
                if !super::is_python_request(&python) {
                    return Err(bad(format!("`{python}` is not a Python version request")));
                }
                let command = plugin
                    .command
                    .ok_or_else(|| bad("`runtime = \"python\"` needs `command`".into()))?;
                if !is_script_path(&command) {
                    return Err(bad(format!(
                        "`{command}` is not a relative script path inside the pack"
                    )));
                }
                Ok(Some(Self {
                    python,
                    command,
                    args: plugin.args,
                }))
            }
            Some(other) => Err(bad(format!(
                "`{other}` is not a runtime; use `native` or `python`"
            ))),
        }
    }

    /// The program and arguments of the plugin spec: the environment's
    /// interpreter, then [`super::PYTHON_FLAGS`], the script and its
    /// arguments. The script path is relative; the plugin runs from its tree.
    #[must_use]
    pub fn launch(&self, env: &ManagedEnv) -> (PathBuf, Vec<String>) {
        let rest: Vec<String> = std::iter::once(self.command.clone())
            .chain(self.args.iter().cloned())
            .collect();
        (env.python(), env.args(&rest))
    }
}

/// What the `PackPath` def of the pack-manifest schema accepts, minus a
/// leading dash (the interpreter would read it as an option): relative,
/// forward slashes, no drive letter, no `..` segment. A NUL cannot reach an
/// argument list, so it is refused as well.
fn is_script_path(command: &str) -> bool {
    let drive =
        command.as_bytes().get(1) == Some(&b':') && command.as_bytes()[0].is_ascii_alphabetic();
    !command.is_empty()
        && !command.starts_with(['-', '/', '\\'])
        && !drive
        && !command.contains(['\\', '\0'])
        && !command.split('/').any(|s| s == "..")
}
