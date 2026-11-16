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

use slopvac_config::{ConfigError, Discovery, Inputs, NoPresets};

/// The `[network]` settings that govern the update check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetworkPolicy {
    /// `[network] offline = true` in the resolved stack.
    pub offline: bool,
    /// `[network] update_check = false` in the resolved stack. `None` when nothing turned it off.
    pub update_check: Option<bool>,
}

impl NetworkPolicy {
    /// The policy for a run that could not read its config: no network at all.
    ///
    /// A config that cannot be read might have said `offline = true`, so the fallback errs
    /// towards staying offline, as an unparsable `SLOPVAC_OFFLINE` does.
    #[must_use]
    pub fn cautious() -> Self {
        Self {
            offline: true,
            update_check: Some(false),
        }
    }
}

/// Why the project config could not be loaded. The CLI exits 2.
#[derive(Debug)]
pub struct PolicyError(ConfigError);

impl PolicyError {
    /// The stable error code: `config-unreadable` for a file that cannot be read, otherwise the
    /// code of the config diagnostic.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match &self.0 {
            ConfigError::Io { .. } => "config-unreadable",
            other => other.code().as_str(),
        }
    }

    /// The message: a read failure in one line, or the rendered diagnostics.
    #[must_use]
    pub fn message(&self) -> String {
        match &self.0 {
            ConfigError::Io { .. } => self.0.to_string(),
            ConfigError::Rejected(r) => r.render(),
        }
    }

    /// Whether the file was read and its content refused (as opposed to not read at all).
    #[must_use]
    pub fn is_content(&self) -> bool {
        matches!(self.0, ConfigError::Rejected(_))
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
    let Some(config) = discovery.nearest(start).map_err(PolicyError)? else {
        return Ok(NetworkPolicy::default());
    };
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
    Ok(NetworkPolicy {
        offline: resolved.is_offline(),
        update_check,
    })
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
    fn content_errors_are_reported_with_a_code_and_no_absolute_path() {
        for body in [
            "{{{",
            "mode = \"prose\"\n",
            "schema_version = 3\n[network]\noffline = \"yes\"\n",
        ] {
            let d = tree(&[("slopvac.toml", body)]);
            let err = network_policy(d.path()).unwrap_err();
            assert!(err.is_content(), "{body}");
            assert!(err.code().starts_with("config-"), "{}", err.code());
            let text = err.message();
            assert!(!text.contains(d.path().to_str().unwrap()), "{text}");
        }
    }

    #[test]
    fn read_failures_are_unreadable_and_name_only_the_file() {
        let d = tree(&[]);
        std::fs::create_dir(d.path().join("slopvac.toml")).unwrap();
        let err = network_policy(d.path()).unwrap_err();
        assert!(!err.is_content());
        assert_eq!(err.code(), "config-unreadable");
        assert_eq!(
            err.message(),
            "cannot read slopvac.toml: not a regular file"
        );
    }

    #[test]
    fn the_cautious_policy_is_offline() {
        let p = NetworkPolicy::cautious();
        assert!(p.offline);
        assert_eq!(p.update_check, Some(false));
    }

    #[test]
    fn a_v3_stamp_alone_is_valid() {
        let d = tree(&[("slopvac.toml", V3)]);
        assert_eq!(policy(d.path()), NetworkPolicy::default());
    }
}
