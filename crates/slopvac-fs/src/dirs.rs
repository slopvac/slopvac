//! Per-user directories.
//!
//! `etcetera` reads the process environment directly and has no injection point. These
//! functions apply its Unix (XDG) rules to an injected lookup instead, so a test can isolate
//! every input. A test below keeps the production result equal to `etcetera`'s.

use std::env::VarError;
use std::path::PathBuf;

use crate::{Error, Result};

/// The environment variable that overrides the cache directory, as in 2.x.
pub const CACHE_DIR_VAR: &str = "SLOPVAC_CACHE_DIR";

const APP: &str = "slopvac";

/// Look up an environment variable. `None` means unset.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

fn process_env(name: &str) -> Option<String> {
    var_to_option(std::env::var(name))
}

/// A variable that is not valid UTF-8 counts as unset: no path is built from it, and the
/// lookup has no error channel. The fallback location is used instead.
#[expect(
    clippy::manual_ok_err,
    reason = "both error variants are named on purpose, to document that NotUnicode is unset"
)]
fn var_to_option(value: std::result::Result<String, VarError>) -> Option<String> {
    match value {
        Ok(value) => Some(value),
        Err(VarError::NotPresent | VarError::NotUnicode(_)) => None,
    }
}

/// The cache directory: `SLOPVAC_CACHE_DIR` when set and non-empty, else `XDG_CACHE_HOME`
/// (absolute), else `~/.cache`, plus `slopvac`.
pub fn cache_dir() -> Result<PathBuf> {
    cache_dir_with(&process_env)
}

/// [`cache_dir`] reading `SLOPVAC_CACHE_DIR`, `XDG_CACHE_HOME` and `HOME` through `lookup`.
pub fn cache_dir_with(lookup: Lookup<'_>) -> Result<PathBuf> {
    if let Some(dir) = lookup(CACHE_DIR_VAR).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    xdg(lookup, "XDG_CACHE_HOME", ".cache")
}

/// The configuration directory: `XDG_CONFIG_HOME` (absolute), else `~/.config`, plus `slopvac`.
pub fn config_dir() -> Result<PathBuf> {
    config_dir_with(&process_env)
}

/// [`config_dir`] reading `XDG_CONFIG_HOME` and `HOME` through `lookup`.
pub fn config_dir_with(lookup: Lookup<'_>) -> Result<PathBuf> {
    xdg(lookup, "XDG_CONFIG_HOME", ".config")
}

/// An XDG base directory: the variable when it is an absolute path, else `$HOME/<default>`.
fn xdg(lookup: Lookup<'_>, var: &str, default: &str) -> Result<PathBuf> {
    let base = match lookup(var).map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(dir) => dir,
        None => home(lookup)?.join(default),
    };
    Ok(base.join(APP))
}

/// `HOME` through the lookup; with no lookup value, the standard library's home directory.
fn home(lookup: Lookup<'_>) -> Result<PathBuf> {
    match lookup("HOME").filter(|h| !h.is_empty()) {
        Some(h) => Ok(PathBuf::from(h)),
        None => etcetera::home_dir().map_err(|_| Error::NoHomeDir),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use etcetera::BaseStrategy;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn override_wins() {
        let e = env(&[
            (CACHE_DIR_VAR, "/custom/c"),
            ("XDG_CACHE_HOME", "/x"),
            ("HOME", "/h"),
        ]);
        assert_eq!(cache_dir_with(&e).unwrap(), Path::new("/custom/c"));
    }

    #[test]
    fn xdg_cache_home_beats_home() {
        let e = env(&[("XDG_CACHE_HOME", "/x"), ("HOME", "/h")]);
        assert_eq!(cache_dir_with(&e).unwrap(), Path::new("/x/slopvac"));
    }

    #[test]
    fn empty_override_and_relative_xdg_fall_back_to_home() {
        let e = env(&[
            (CACHE_DIR_VAR, ""),
            ("XDG_CACHE_HOME", "rel"),
            ("HOME", "/h"),
        ]);
        assert_eq!(cache_dir_with(&e).unwrap(), Path::new("/h/.cache/slopvac"));
    }

    #[test]
    fn config_dir_cases() {
        let e = env(&[("XDG_CONFIG_HOME", "/xc"), ("HOME", "/h")]);
        assert_eq!(config_dir_with(&e).unwrap(), Path::new("/xc/slopvac"));
        let e = env(&[("HOME", "/h")]);
        assert_eq!(
            config_dir_with(&e).unwrap(),
            Path::new("/h/.config/slopvac")
        );
    }

    #[test]
    fn xdg_rules_match_etcetera() {
        // The process environment minus the slopvac override, which etcetera does not know.
        let no_override = |n: &str| {
            if n == CACHE_DIR_VAR {
                None
            } else {
                process_env(n)
            }
        };
        let s = etcetera::choose_base_strategy().unwrap();
        assert_eq!(
            cache_dir_with(&no_override).unwrap(),
            s.cache_dir().join(APP)
        );
        assert_eq!(config_dir().unwrap(), s.config_dir().join(APP));
    }

    #[test]
    fn non_utf8_variable_counts_as_unset() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let bad = Err(VarError::NotUnicode(OsString::from_vec(vec![0xff])));
        assert_eq!(var_to_option(bad), None);
        assert_eq!(var_to_option(Err(VarError::NotPresent)), None);
        assert_eq!(var_to_option(Ok("v".to_owned())), Some("v".to_owned()));
    }

    #[test]
    fn unset_process_variable_is_none() {
        assert_eq!(process_env("SLOPVAC_FS_TEST_SURELY_UNSET"), None);
    }
}
