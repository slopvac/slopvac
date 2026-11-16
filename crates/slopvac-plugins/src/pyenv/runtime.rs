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
    /// The interpreter request as a PEP 440 specifier set that uv takes, for
    /// example `3.12` or `>=3.12,<4` (see [`pep440_specifier`]). The environment
    /// store keys on this value.
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
                let python = pep440_specifier(&python).map_err(bad)?;
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

/// Translate a `python` value of the manifest (the `VersionReq` def of the
/// pack-manifest schema, semver rules) into the PEP 440 specifier set that uv
/// takes as `--python`.
///
/// The grammar is comma-separated comparators, each an optional operator
/// (`^ ~ = >= <= > <`) and a version of one to three numeric parts, with an
/// optional `-` pre-release suffix. Rules, with semver's partial-version
/// meaning:
///
/// | Manifest | PEP 440 |
/// | --- | --- |
/// | `3.12`, `3.12.4` alone | unchanged (uv reads `3.12` as any 3.12.x) |
/// | `=3.12`, `=3`, `=3.12.4` | `==3.12.*`, `==3.*`, `==3.12.4` |
/// | `^3.12`, `^3`, `^0.2.3`, `^0.0.3` | `>=3.12,<4`, `>=3,<4`, `>=0.2.3,<0.3`, `>=0.0.3,<0.0.4` |
/// | `~3.12`, `~3.12.4`, `~3` | `>=3.12,<3.13`, `>=3.12.4,<3.13`, `>=3,<4` |
/// | `>=3.12`, `<3.13` | unchanged |
/// | `>3.12`, `>3`, `>3.12.4` | `>=3.13`, `>=4`, `>3.12.4` |
/// | `<=3.12`, `<=3`, `<=3.12.4` | `<3.13`, `<4`, `<=3.12.4` |
///
/// In a list, a bare version means `=`. Comma-separated parts join with `,`.
///
/// # Errors
/// A message naming the value, for a pre-release suffix (PEP 440 interpreter
/// requests cannot express it), a part that is not a number, a list entry
/// that is empty, a number too large, or a result over 64 bytes.
pub fn pep440_specifier(req: &str) -> Result<String, String> {
    let parts: Vec<&str> = req.split(',').map(str::trim).collect();
    if let [only] = parts[..]
        && let Ok((None, _)) = comparator(only)
    {
        return Ok(only.to_owned());
    }
    let mut out = Vec::new();
    for part in &parts {
        let (op, v) = comparator(part).map_err(|why| format!("`{req}`: {why}"))?;
        out.extend(clauses(op.unwrap_or("="), &v).map_err(|why| format!("`{req}`: {why}"))?);
    }
    let joined = out.join(",");
    if joined.len() > 64 {
        return Err(format!("`{req}` translates to more than 64 bytes"));
    }
    Ok(joined)
}

fn comparator(part: &str) -> Result<(Option<&'static str>, Vec<u64>), String> {
    let (op, rest) = ["<=", ">=", "^", "~", "=", ">", "<"]
        .iter()
        .find_map(|o| part.strip_prefix(o).map(|r| (Some(*o), r)))
        .unwrap_or((None, part));
    let rest = rest.trim();
    if rest.contains('-') {
        return Err(format!(
            "`{part}` has a pre-release suffix, which a Python version request cannot express"
        ));
    }
    let nums: Vec<u64> = rest
        .split('.')
        .map(|n| {
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("`{part}` is not a version of one to three numbers"));
            }
            n.parse()
                .map_err(|_| format!("`{part}` has a number that is too large"))
        })
        .collect::<Result<_, _>>()?;
    if nums.is_empty() || nums.len() > 3 {
        return Err(format!("`{part}` is not a version of one to three numbers"));
    }
    Ok((op, nums))
}

fn dotted(v: &[u64]) -> String {
    v.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

/// `v` with the number at `idx` raised by one and the later parts dropped.
fn bumped(v: &[u64], idx: usize) -> Result<String, String> {
    let mut head = v[..=idx].to_vec();
    head[idx] = head[idx]
        .checked_add(1)
        .ok_or_else(|| "a number is too large".to_owned())?;
    Ok(dotted(&head))
}

fn clauses(op: &str, v: &[u64]) -> Result<Vec<String>, String> {
    let n = v.len();
    let ver = dotted(v);
    Ok(match op {
        "=" if n == 3 => vec![format!("=={ver}")],
        "=" => vec![format!("=={ver}.*")],
        ">=" | "<" => vec![format!("{op}{ver}")],
        ">" if n == 3 => vec![format!(">{ver}")],
        ">" => vec![format!(">={}", bumped(v, n - 1)?)],
        "<=" if n == 3 => vec![format!("<={ver}")],
        "<=" => vec![format!("<{}", bumped(v, n - 1)?)],
        "~" => vec![
            format!(">={ver}"),
            format!("<{}", bumped(v, usize::from(n > 1))?),
        ],
        // "^": the leftmost non-zero number stays; a trailing all-zero version
        // keeps its last given number.
        _ => {
            let idx = v
                .iter()
                .position(|x| *x != 0)
                .map_or(n - 1, |i| i.min(n - 1));
            vec![format!(">={ver}"), format!("<{}", bumped(v, idx)?)]
        }
    })
}
