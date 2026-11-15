//! Project network policy from `slopvac.toml`.
//!
//! STUB(slopvac-8l4.12.18): `crates/slopvac-config` is still a placeholder, so this reads only
//! `[network] offline` and `[network] update_check` from the nearest `slopvac.toml`. It follows
//! the discovery rule of `docs/contract/config.md` (walk up from the start directory, stop after
//! the directory that holds `.git`). It ignores `extends` chains and an unparsable file. The real
//! config layer replaces it, and this module goes away.

use std::path::Path;

/// The `[network]` settings that govern the update check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetworkPolicy {
    /// `[network] offline = true`. Offline is sticky, so `false` and unset mean the same.
    pub offline: bool,
    /// `[network] update_check`, when set.
    pub update_check: Option<bool>,
}

/// Read the policy of the nearest `slopvac.toml` at or above `start`.
#[must_use]
pub fn network_policy(start: &Path) -> NetworkPolicy {
    for dir in start.ancestors() {
        let file = dir.join("slopvac.toml");
        if let Ok(text) = std::fs::read_to_string(&file) {
            return parse(&text);
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    NetworkPolicy::default()
}

fn parse(text: &str) -> NetworkPolicy {
    let Ok(doc) = text.parse::<toml::Table>() else {
        return NetworkPolicy::default();
    };
    let Some(network) = doc.get("network").and_then(toml::Value::as_table) else {
        return NetworkPolicy::default();
    };
    NetworkPolicy {
        offline: network.get("offline").and_then(toml::Value::as_bool) == Some(true),
        update_check: network.get("update_check").and_then(toml::Value::as_bool),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, body) in files {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        dir
    }

    #[test]
    fn reads_both_keys() {
        let d = tree(&[(
            "slopvac.toml",
            "schema_version = 3\n[network]\noffline = true\nupdate_check = false\n",
        )]);
        assert_eq!(
            network_policy(d.path()),
            NetworkPolicy {
                offline: true,
                update_check: Some(false)
            }
        );
    }

    #[test]
    fn walks_up_to_the_nearest_file() {
        let d = tree(&[
            ("slopvac.toml", "[network]\noffline = true\n"),
            ("a/b/keep", ""),
        ]);
        assert!(network_policy(&d.path().join("a/b")).offline);
    }

    #[test]
    fn nearest_file_wins_over_a_farther_one() {
        let d = tree(&[
            ("slopvac.toml", "[network]\noffline = true\n"),
            ("a/slopvac.toml", "schema_version = 3\n"),
        ]);
        assert!(!network_policy(&d.path().join("a")).offline);
    }

    #[test]
    fn the_walk_stops_after_the_git_root() {
        let d = tree(&[
            ("slopvac.toml", "[network]\noffline = true\n"),
            ("repo/.git/HEAD", ""),
            ("repo/src/keep", ""),
        ]);
        assert_eq!(
            network_policy(&d.path().join("repo/src")),
            NetworkPolicy::default()
        );
    }

    #[test]
    fn a_config_beside_git_still_counts() {
        let d = tree(&[
            ("repo/.git/HEAD", ""),
            ("repo/slopvac.toml", "[network]\nupdate_check = false\n"),
        ]);
        assert_eq!(
            network_policy(&d.path().join("repo")).update_check,
            Some(false)
        );
    }

    #[test]
    fn unparsable_or_silent_files_set_nothing() {
        for body in [
            "{{{",
            "schema_version = 3\n",
            "[network]\noffline = \"yes\"\n",
        ] {
            let d = tree(&[("slopvac.toml", body)]);
            assert_eq!(network_policy(d.path()), NetworkPolicy::default(), "{body}");
        }
    }
}
