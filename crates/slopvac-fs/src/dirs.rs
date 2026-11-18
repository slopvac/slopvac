//! Per-user directories.
//!
//! The rules follow the XDG base directory specification on Unix. Every input, the home
//! directory fallback included, is injected, so a test can isolate the process environment.
//!
//! The functions accept UTF-8 only: a variable that is set but not valid UTF-8 is an
//! [`Error::NotUtf8Env`]. This is a recorded departure (see the engineering standards).

use std::env::VarError;
use std::path::PathBuf;

use crate::{Error, Result};

/// The environment variable that overrides the cache directory, as in 2.x.
pub const CACHE_DIR_VAR: &str = "SLOPVAC_CACHE_DIR";

const APP: &str = "slopvac";

/// Look up an environment variable, as `std::env::var` does.
///
/// A value that is not valid UTF-8 is `Err(VarError::NotUnicode(_))`, which the functions here
/// turn into [`Error::NotUtf8Env`].
pub type Lookup<'a> = &'a dyn Fn(&str) -> std::result::Result<String, VarError>;

/// The home directory to use when `HOME` is unset or empty. `None` means no home is known.
pub type HomeFallback<'a> = &'a dyn Fn() -> Option<PathBuf>;

fn process_env(name: &str) -> std::result::Result<String, VarError> {
    std::env::var(name)
}

fn process_home() -> Option<PathBuf> {
    etcetera::home_dir().ok()
}

/// The value of a variable, or `None` when it is unset. A value that is not UTF-8 is an error.
fn value(lookup: Lookup<'_>, name: &'static str) -> Result<Option<String>> {
    match lookup(name) {
        Ok(v) => Ok(Some(v)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(Error::NotUtf8Env { var: name }),
    }
}

/// The cache directory: `SLOPVAC_CACHE_DIR` when set and non-empty, else `XDG_CACHE_HOME`
/// (absolute), else `~/.cache`, plus `slopvac`.
pub fn cache_dir() -> Result<PathBuf> {
    cache_dir_with(&process_env, &process_home)
}

/// [`cache_dir`] reading `SLOPVAC_CACHE_DIR`, `XDG_CACHE_HOME` and `HOME` through `lookup`, and
/// asking `home_fallback` when `HOME` is unset or empty.
pub fn cache_dir_with(lookup: Lookup<'_>, home_fallback: HomeFallback<'_>) -> Result<PathBuf> {
    if let Some(dir) = value(lookup, CACHE_DIR_VAR)?.filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    xdg(lookup, home_fallback, "XDG_CACHE_HOME", ".cache")
}

/// The configuration directory: `XDG_CONFIG_HOME` (absolute), else `~/.config`, plus `slopvac`.
pub fn config_dir() -> Result<PathBuf> {
    config_dir_with(&process_env, &process_home)
}

/// [`config_dir`] reading `XDG_CONFIG_HOME` and `HOME` through `lookup`, and asking
/// `home_fallback` when `HOME` is unset or empty.
pub fn config_dir_with(lookup: Lookup<'_>, home_fallback: HomeFallback<'_>) -> Result<PathBuf> {
    xdg(lookup, home_fallback, "XDG_CONFIG_HOME", ".config")
}

/// An XDG base directory: the variable when it is an absolute path, else `$HOME/<default>`.
fn xdg(
    lookup: Lookup<'_>,
    home_fallback: HomeFallback<'_>,
    var: &'static str,
    default: &str,
) -> Result<PathBuf> {
    let base = match value(lookup, var)?
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        Some(dir) => dir,
        None => home(lookup, home_fallback)?.join(default),
    };
    Ok(base.join(APP))
}

/// `HOME` through the lookup. When `HOME` is unset or empty, `home_fallback` answers.
fn home(lookup: Lookup<'_>, home_fallback: HomeFallback<'_>) -> Result<PathBuf> {
    match value(lookup, "HOME")?.filter(|h| !h.is_empty()) {
        Some(h) => Ok(PathBuf::from(h)),
        None => home_fallback().ok_or(Error::NoHomeDir),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> std::result::Result<String, VarError> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned().ok_or(VarError::NotPresent)
    }

    fn no_home() -> Option<PathBuf> {
        None
    }

    #[test]
    fn override_wins() {
        let e = env(&[
            (CACHE_DIR_VAR, "/custom/c"),
            ("XDG_CACHE_HOME", "/x"),
            ("HOME", "/h"),
        ]);
        assert_eq!(
            cache_dir_with(&e, &no_home).unwrap(),
            Path::new("/custom/c")
        );
    }

    #[test]
    fn xdg_cache_home_beats_home() {
        let e = env(&[("XDG_CACHE_HOME", "/x"), ("HOME", "/h")]);
        assert_eq!(
            cache_dir_with(&e, &no_home).unwrap(),
            Path::new("/x/slopvac")
        );
    }

    #[test]
    fn empty_override_and_relative_xdg_fall_back_to_home() {
        let e = env(&[
            (CACHE_DIR_VAR, ""),
            ("XDG_CACHE_HOME", "rel"),
            ("HOME", "/h"),
        ]);
        assert_eq!(
            cache_dir_with(&e, &no_home).unwrap(),
            Path::new("/h/.cache/slopvac")
        );
    }

    #[test]
    fn config_dir_cases() {
        let e = env(&[("XDG_CONFIG_HOME", "/xc"), ("HOME", "/h")]);
        assert_eq!(
            config_dir_with(&e, &no_home).unwrap(),
            Path::new("/xc/slopvac")
        );
        let e = env(&[("HOME", "/h")]);
        assert_eq!(
            config_dir_with(&e, &no_home).unwrap(),
            Path::new("/h/.config/slopvac")
        );
    }

    #[test]
    fn unset_or_empty_home_asks_only_the_injected_fallback() {
        // The real HOME of this process is never consulted, whatever it is.
        let fallback = || Some(PathBuf::from("/injected"));
        for e in [env(&[]), env(&[("HOME", "")])] {
            assert_eq!(
                cache_dir_with(&e, &fallback).unwrap(),
                Path::new("/injected/.cache/slopvac")
            );
            assert_eq!(
                config_dir_with(&e, &fallback).unwrap(),
                Path::new("/injected/.config/slopvac")
            );
            assert!(matches!(
                cache_dir_with(&e, &no_home),
                Err(Error::NoHomeDir)
            ));
        }
    }

    #[test]
    fn set_home_does_not_ask_the_fallback() {
        let e = env(&[("HOME", "/h")]);
        let fallback = || -> Option<PathBuf> { panic!("fallback must not run") };
        assert_eq!(
            config_dir_with(&e, &fallback).unwrap(),
            Path::new("/h/.config/slopvac")
        );
    }

    fn not_unicode() -> VarError {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        VarError::NotUnicode(OsString::from_vec(vec![0xff]))
    }

    #[test]
    fn non_utf8_value_is_an_error_naming_the_variable() {
        for bad in [CACHE_DIR_VAR, "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "HOME"] {
            let e = |n: &str| {
                if n == bad {
                    Err(not_unicode())
                } else if n == "HOME" {
                    Ok("/h".to_owned())
                } else {
                    Err(VarError::NotPresent)
                }
            };
            // XDG_CACHE_HOME and XDG_CONFIG_HOME each affect one resolver only, so try both.
            let cache = cache_dir_with(&e, &no_home).err();
            let config = config_dir_with(&e, &no_home).err();
            let hit = [cache, config].into_iter().flatten().next();
            assert!(
                matches!(hit, Some(Error::NotUtf8Env { var }) if var == bad),
                "{bad}: {hit:?}"
            );
        }
    }

    #[test]
    fn non_utf8_error_text_names_the_variable_only() {
        let e = |_: &str| Err(not_unicode());
        let err = config_dir_with(&e, &no_home).unwrap_err();
        assert_eq!(
            err.to_string(),
            "environment variable XDG_CONFIG_HOME is not valid UTF-8"
        );
    }

    #[test]
    fn non_utf8_home_does_not_matter_when_xdg_is_valid() {
        let e = |n: &str| match n {
            "HOME" => Err(not_unicode()),
            "XDG_CACHE_HOME" => Ok("/x".to_owned()),
            "XDG_CONFIG_HOME" => Ok("/xc".to_owned()),
            _ => Err(VarError::NotPresent),
        };
        assert_eq!(
            cache_dir_with(&e, &no_home).unwrap(),
            Path::new("/x/slopvac")
        );
        assert_eq!(
            config_dir_with(&e, &no_home).unwrap(),
            Path::new("/xc/slopvac")
        );
    }
}
