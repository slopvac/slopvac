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

/// Look up an environment variable, as `std::env::var` does.
///
/// A value that is not valid UTF-8 is `Err(VarError::NotUnicode(_))`. No path is built from it:
/// it counts as unset, except for `HOME`, where it is an error (see [`Error::NoHomeDir`]).
pub type Lookup<'a> = &'a dyn Fn(&str) -> std::result::Result<String, VarError>;

fn process_env(name: &str) -> std::result::Result<String, VarError> {
    std::env::var(name)
}

/// The value of a variable, or `None` when it is unset or not valid UTF-8.
#[expect(
    clippy::manual_ok_err,
    reason = "both error variants are named on purpose, to document that NotUnicode is unset"
)]
fn value(lookup: Lookup<'_>, name: &str) -> Option<String> {
    match lookup(name) {
        Ok(v) => Some(v),
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
    if let Some(dir) = value(lookup, CACHE_DIR_VAR).filter(|v| !v.is_empty()) {
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
    let base = match value(lookup, var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        Some(dir) => dir,
        None => home(lookup)?.join(default),
    };
    Ok(base.join(APP))
}

/// `HOME` through the lookup. When `HOME` is unset or empty, the standard library's home
/// directory answers. When `HOME` is set but not valid UTF-8 the result is an error, because
/// the fallback would read the same rejected value from the process environment again.
fn home(lookup: Lookup<'_>) -> Result<PathBuf> {
    match lookup("HOME") {
        Ok(h) if !h.is_empty() => Ok(PathBuf::from(h)),
        Ok(_) | Err(VarError::NotPresent) => etcetera::home_dir().map_err(|_| Error::NoHomeDir),
        Err(VarError::NotUnicode(_)) => Err(Error::NoHomeDir),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use etcetera::BaseStrategy;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> std::result::Result<String, VarError> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned().ok_or(VarError::NotPresent)
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
                Err(VarError::NotPresent)
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

    fn not_unicode() -> VarError {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        VarError::NotUnicode(OsString::from_vec(vec![0xff]))
    }

    #[test]
    fn non_utf8_override_and_xdg_count_as_unset() {
        let bad = |n: &str| match n {
            CACHE_DIR_VAR | "XDG_CACHE_HOME" | "XDG_CONFIG_HOME" => Err(not_unicode()),
            "HOME" => Ok("/h".to_owned()),
            _ => Err(VarError::NotPresent),
        };
        assert_eq!(
            cache_dir_with(&bad).unwrap(),
            Path::new("/h/.cache/slopvac")
        );
        assert_eq!(
            config_dir_with(&bad).unwrap(),
            Path::new("/h/.config/slopvac")
        );
    }

    #[test]
    fn non_utf8_home_is_an_error_and_never_falls_back() {
        let bad_home = |n: &str| match n {
            "HOME" => Err(not_unicode()),
            _ => Err(VarError::NotPresent),
        };
        assert!(matches!(cache_dir_with(&bad_home), Err(Error::NoHomeDir)));
        assert!(matches!(config_dir_with(&bad_home), Err(Error::NoHomeDir)));
    }

    #[test]
    fn non_utf8_home_does_not_matter_when_xdg_is_set() {
        let e = |n: &str| match n {
            "HOME" => Err(not_unicode()),
            "XDG_CACHE_HOME" => Ok("/x".to_owned()),
            _ => Err(VarError::NotPresent),
        };
        assert_eq!(cache_dir_with(&e).unwrap(), Path::new("/x/slopvac"));
    }

    #[test]
    fn unset_process_variable_is_not_present() {
        assert!(matches!(
            process_env("SLOPVAC_FS_TEST_SURELY_UNSET"),
            Err(VarError::NotPresent)
        ));
    }
}
