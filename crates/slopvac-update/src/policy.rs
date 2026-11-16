//! Project network policy, read through the config layer.
//!
//! `[network] offline` and `[network] update_check` come from the resolved `slopvac.toml` stack,
//! so `extends` chains and layer precedence apply, and offline stays sticky: once any layer sets
//! it, no layer can unset it.
//!
//! The read runs before argument parsing, on files the user did not name, so the loader reads
//! only regular files, without blocking, and at most 1 MiB each (`slopvac_config::read`). Error
//! messages name files by their name or by their path relative to the project root, never by an
//! absolute path.

use std::path::Path;

use slopvac_config::{Code, ConfigError, Discovery, Inputs, LoadedConfig, NoPresets};

/// Why a config was refused, kept for the commands that need the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// A stable code: `config-unreadable` for an unreadable `extends` target, otherwise the
    /// config diagnostic code.
    pub code: &'static str,
    /// One line that names the file by name or by path relative to the project root.
    pub message: String,
}

/// The `[network]` settings that govern the update check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkPolicy {
    /// Set when the stack was refused. The policy then fails closed: offline, no update check. A
    /// command that needs the network stops with this error. Local commands still run.
    pub refused: Option<Refusal>,
    /// `[network] offline = true` in the resolved stack.
    pub offline: bool,
    /// `[network] update_check = false` in the resolved stack. `None` when nothing turned it off.
    pub update_check: Option<bool>,
}

/// Why the project config could not be read. The CLI exits 2.
///
/// Only a failure to read the nearest config is an error here. A stack that was refused (a 2.x
/// file, an unknown key, an `extends` target that is missing or unreadable) is an `Ok` policy that
/// fails closed and carries the [`Refusal`]: `migrate`, `doctor` and the help still run.
#[derive(Debug)]
pub struct PolicyError(ConfigError);

impl PolicyError {
    /// The stable error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        "config-unreadable"
    }

    /// One line, naming the file by name or by path relative to the project root.
    #[must_use]
    pub fn message(&self) -> String {
        self.0.to_string()
    }
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for PolicyError {}

/// Load the config that applies to `start` and read its network settings.
///
/// With no `slopvac.toml` on the path the defaults apply.
pub fn network_policy(start: &Path) -> Result<NetworkPolicy, PolicyError> {
    let discovery = Discovery::new(&NoPresets);
    let config = match discovery.nearest(start) {
        Ok(Some(config)) => config,
        Ok(None) => return Ok(NetworkPolicy::default()),
        Err(ConfigError::Rejected(rejected)) => {
            let first = rejected.diagnostics.first().map(|d| d.code);
            let code = match first {
                Some(Code::ExtendsMissing) => "config-unreadable",
                Some(other) => other.as_str(),
                None => "config-invalid",
            };
            let message = ConfigError::Rejected(rejected).to_string();
            return Ok(NetworkPolicy {
                refused: Some(Refusal { code, message }),
                offline: true,
                update_check: Some(false),
            });
        }
        Err(err) => return Err(PolicyError(err)),
    };
    Ok(policy_of(&config))
}

/// The network settings of an already loaded config.
#[must_use]
pub fn policy_of(config: &LoadedConfig) -> NetworkPolicy {
    let resolved = config.resolve(
        Path::new(""),
        &Inputs {
            presets: &NoPresets,
            front_matter: &[],
            directives: &[],
            cli: &[],
            env: &[],
        },
    );
    let update_check = match resolved.get(&["network", "update_check"]) {
        Some(serde_json::Value::Bool(false)) => Some(false),
        _ => None,
    };
    NetworkPolicy {
        refused: None,
        offline: resolved.is_offline(),
        update_check,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        // A `.git` entry stops the upward walk inside the temp directory.
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        for (path, body) in files {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        dir
    }

    const V3: &str = "schema_version = 3\n";

    fn policy(dir: &Path) -> NetworkPolicy {
        network_policy(dir).expect("loads")
    }

    #[test]
    fn no_config_means_defaults() {
        assert_eq!(policy(tree(&[]).path()), NetworkPolicy::default());
    }

    #[test]
    fn reads_both_keys() {
        let d = tree(&[(
            "slopvac.toml",
            "schema_version = 3\n[network]\noffline = true\nupdate_check = false\n",
        )]);
        assert_eq!(
            policy(d.path()),
            NetworkPolicy {
                refused: None,
                offline: true,
                update_check: Some(false)
            }
        );
    }

    #[test]
    fn an_extended_file_can_turn_offline_on_and_the_project_cannot_turn_it_off() {
        let d = tree(&[
            (
                "base.toml",
                "schema_version = 3\n[network]\noffline = true\n",
            ),
            (
                "slopvac.toml",
                "schema_version = 3\nextends = [\"base.toml\"]\n[network]\noffline = false\n",
            ),
        ]);
        assert!(policy(d.path()).offline);
    }

    #[test]
    fn the_project_overrides_update_check_from_its_base() {
        let d = tree(&[
            (
                "base.toml",
                "schema_version = 3\n[network]\nupdate_check = false\n",
            ),
            (
                "slopvac.toml",
                "schema_version = 3\nextends = [\"base.toml\"]\n[network]\nupdate_check = true\n",
            ),
        ]);
        assert_eq!(policy(d.path()).update_check, None);
        let d = tree(&[
            (
                "base.toml",
                "schema_version = 3\n[network]\nupdate_check = true\n",
            ),
            (
                "slopvac.toml",
                "schema_version = 3\nextends = [\"base.toml\"]\n[network]\nupdate_check = false\n",
            ),
        ]);
        assert_eq!(policy(d.path()).update_check, Some(false));
    }

    #[test]
    fn walks_up_to_the_nearest_file_and_stops_after_the_git_root() {
        let d = tree(&[
            (
                "slopvac.toml",
                "schema_version = 3\n[network]\noffline = true\n",
            ),
            ("a/b/keep", ""),
        ]);
        std::fs::remove_dir(d.path().join(".git")).unwrap();
        assert!(policy(&d.path().join("a/b")).offline);
        std::fs::create_dir(d.path().join("a/.git")).unwrap();
        assert_eq!(policy(&d.path().join("a/b")), NetworkPolicy::default());
    }

    #[test]
    fn a_refused_stack_fails_closed_with_the_reason() {
        for (files, code) in [
            (vec![("slopvac.toml", "{{{")], "config-parse"),
            (
                vec![("slopvac.toml", "mode = \"prose\"\n")],
                "config-legacy-version",
            ),
            (
                vec![(
                    "slopvac.toml",
                    "schema_version = 3\nextends = [\"nope.toml\"]\n[network]\noffline = true\n",
                )],
                "config-unreadable",
            ),
            (
                vec![
                    ("bad.toml", "schema_version = 3\nbogus = 1\n"),
                    (
                        "slopvac.toml",
                        "schema_version = 3\nextends = [\"bad.toml\"]\n",
                    ),
                ],
                "config-unknown-key",
            ),
        ] {
            let d = tree(&files);
            let p = policy(d.path());
            assert!(p.offline, "{code}");
            assert_eq!(p.update_check, Some(false), "{code}");
            let refusal = p.refused.expect("refused");
            assert_eq!(refusal.code, code);
            assert!(
                !refusal.message.contains(d.path().to_str().unwrap()),
                "{}",
                refusal.message
            );
        }
    }

    #[test]
    fn read_failures_are_unreadable_and_name_only_the_file() {
        let d = tree(&[]);
        std::fs::create_dir(d.path().join("slopvac.toml")).unwrap();
        let err = network_policy(d.path()).unwrap_err();
        assert_eq!(err.code(), "config-unreadable");
        assert_eq!(
            err.message(),
            "cannot read slopvac.toml: not a regular file"
        );
    }

    #[test]
    fn a_v3_stamp_alone_is_valid() {
        let d = tree(&[("slopvac.toml", V3)]);
        assert_eq!(policy(d.path()), NetworkPolicy::default());
    }
}
