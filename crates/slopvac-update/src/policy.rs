//! Project network policy from `slopvac.toml`.
//!
//! STUB(slopvac-8l4.12.18): `crates/slopvac-config` is still a placeholder, so this reads only
//! `[network] offline` and `[network] update_check` from the nearest `slopvac.toml`. It follows
//! the discovery rule of `docs/contract/config.md` (walk up from the start directory, stop after
//! the directory that holds `.git`). It ignores `extends` chains and an unparsable file. The real
//! config layer replaces it, and this module goes away.
//!
//! The read runs before argument parsing, on a path the user did not choose, so it is bounded: the
//! file must be a regular file (checked on the open handle, which closes the swap-after-check gap),
//! it opens without blocking, and it reads at most [`MAX_CONFIG_BYTES`].

use std::io::Read;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Largest `slopvac.toml` this stub reads.
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// The `[network]` settings that govern the update check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetworkPolicy {
    /// `[network] offline = true`. Offline is sticky, so `false` and unset mean the same.
    pub offline: bool,
    /// `[network] update_check`, when set.
    pub update_check: Option<bool>,
}

/// Why the nearest `slopvac.toml` could not be read. The CLI exits 2.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    /// The path is a directory, FIFO, device or other non-regular file.
    #[error("{}: not a regular file", .0.display())]
    NotRegular(PathBuf),
    /// The file is larger than [`MAX_CONFIG_BYTES`].
    #[error("{}: larger than {MAX_CONFIG_BYTES} bytes", .0.display())]
    TooLarge(PathBuf),
    /// Any other read failure.
    #[error("{}: cannot read: {reason}", path.display())]
    Unreadable {
        /// The file.
        path: PathBuf,
        /// The I/O error text.
        reason: String,
    },
}

impl PolicyError {
    /// The file the error is about.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::NotRegular(p) | Self::TooLarge(p) => p,
            Self::Unreadable { path, .. } => path,
        }
    }

    /// What went wrong, without the path.
    #[must_use]
    pub fn problem(&self) -> String {
        match self {
            Self::NotRegular(_) => "not a regular file".to_owned(),
            Self::TooLarge(_) => format!("larger than {MAX_CONFIG_BYTES} bytes"),
            Self::Unreadable { reason, .. } => format!("cannot read: {reason}"),
        }
    }
}

/// Read the policy of the nearest `slopvac.toml` at or above `start`.
pub fn network_policy(start: &Path) -> Result<NetworkPolicy, PolicyError> {
    for dir in start.ancestors() {
        let file = dir.join("slopvac.toml");
        if let Some(text) = read_bounded(&file)? {
            return Ok(parse(&text));
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    Ok(NetworkPolicy::default())
}

/// The file's text, or `None` when it does not exist.
fn read_bounded(file: &Path) -> Result<Option<String>, PolicyError> {
    let unreadable = |e: &std::io::Error| PolicyError::Unreadable {
        path: file.to_owned(),
        reason: e.to_string(),
    };
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO with no writer would block an ordinary open.
        opts.custom_flags(i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).unwrap_or_default());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_BACKUP_SEMANTICS lets a directory open, so the handle check names it.
        opts.custom_flags(0x0200_0000);
    }
    let handle = match opts.open(file) {
        Ok(h) => h,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(unreadable(&e)),
    };
    if !handle.metadata().map_err(|e| unreadable(&e))?.is_file() {
        return Err(PolicyError::NotRegular(file.to_owned()));
    }
    let mut text = String::new();
    handle
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| unreadable(&e))?;
    if text.len() as u64 > MAX_CONFIG_BYTES {
        return Err(PolicyError::TooLarge(file.to_owned()));
    }
    Ok(Some(text))
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

    fn policy(dir: &Path) -> NetworkPolicy {
        network_policy(dir).expect("readable")
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
    fn walks_up_to_the_nearest_file() {
        let d = tree(&[
            ("slopvac.toml", "[network]\noffline = true\n"),
            ("a/b/keep", ""),
        ]);
        assert!(policy(&d.path().join("a/b")).offline);
    }

    #[test]
    fn nearest_file_wins_over_a_farther_one() {
        let d = tree(&[
            ("slopvac.toml", "[network]\noffline = true\n"),
            ("a/slopvac.toml", "schema_version = 3\n"),
        ]);
        assert!(!policy(&d.path().join("a")).offline);
    }

    #[test]
    fn the_walk_stops_after_the_git_root() {
        let d = tree(&[
            ("slopvac.toml", "[network]\noffline = true\n"),
            ("repo/.git/HEAD", ""),
            ("repo/src/keep", ""),
        ]);
        assert_eq!(policy(&d.path().join("repo/src")), NetworkPolicy::default());
    }

    #[test]
    fn a_config_beside_git_still_counts() {
        let d = tree(&[
            ("repo/.git/HEAD", ""),
            ("repo/slopvac.toml", "[network]\nupdate_check = false\n"),
        ]);
        assert_eq!(policy(&d.path().join("repo")).update_check, Some(false));
    }

    #[test]
    fn unparsable_or_silent_files_set_nothing() {
        for body in [
            "{{{",
            "schema_version = 3\n",
            "[network]\noffline = \"yes\"\n",
        ] {
            let d = tree(&[("slopvac.toml", body)]);
            assert_eq!(policy(d.path()), NetworkPolicy::default(), "{body}");
        }
    }

    #[test]
    fn a_file_at_the_size_cap_is_read_and_one_byte_more_is_refused() {
        let prefix = "[network]\noffline = true\n#";
        let at_cap = format!(
            "{prefix}{}",
            "a".repeat(usize::try_from(MAX_CONFIG_BYTES).unwrap() - prefix.len())
        );
        let d = tree(&[("slopvac.toml", &at_cap)]);
        assert!(policy(d.path()).offline);

        let over = format!("{at_cap}a");
        let d = tree(&[("slopvac.toml", &over)]);
        assert_eq!(
            network_policy(d.path()),
            Err(PolicyError::TooLarge(d.path().join("slopvac.toml")))
        );
    }

    #[test]
    fn a_directory_named_slopvac_toml_is_refused() {
        let d = tree(&[("slopvac.toml/inner", "")]);
        assert_eq!(
            network_policy(d.path()),
            Err(PolicyError::NotRegular(d.path().join("slopvac.toml")))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let d = tree(&[]);
        let fifo = d.path().join("slopvac.toml");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo");
        assert!(status.success());
        assert_eq!(network_policy(d.path()), Err(PolicyError::NotRegular(fifo)));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_device_is_refused_and_one_to_a_regular_file_is_followed() {
        let d = tree(&[("real.toml", "[network]\noffline = true\n")]);
        let link = d.path().join("slopvac.toml");
        std::os::unix::fs::symlink("/dev/zero", &link).unwrap();
        assert_eq!(
            network_policy(d.path()),
            Err(PolicyError::NotRegular(link.clone()))
        );
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(d.path().join("real.toml"), &link).unwrap();
        assert!(policy(d.path()).offline);
    }
}
