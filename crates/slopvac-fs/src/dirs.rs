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
    resolve(DirKind::Cache, lookup, home_fallback)
}

/// The configuration directory: `XDG_CONFIG_HOME` (absolute), else `~/.config`, plus `slopvac`.
pub fn config_dir() -> Result<PathBuf> {
    config_dir_with(&process_env, &process_home)
}

/// [`config_dir`] reading `XDG_CONFIG_HOME` and `HOME` through `lookup`, and asking
/// `home_fallback` when `HOME` is unset or empty.
pub fn config_dir_with(lookup: Lookup<'_>, home_fallback: HomeFallback<'_>) -> Result<PathBuf> {
    resolve(DirKind::Config, lookup, home_fallback)
}

/// The data directory, which is the plugin store root: `XDG_DATA_HOME` (absolute), else
/// `~/.local/share`, plus `slopvac`.
pub fn data_dir() -> Result<PathBuf> {
    data_dir_with(&process_env, &process_home)
}

/// [`data_dir`] reading `XDG_DATA_HOME` and `HOME` through `lookup`, and asking `home_fallback`
/// when `HOME` is unset or empty.
pub fn data_dir_with(lookup: Lookup<'_>, home_fallback: HomeFallback<'_>) -> Result<PathBuf> {
    resolve(DirKind::Data, lookup, home_fallback)
}

/// The state directory, which holds the trust store: `XDG_STATE_HOME` (absolute), else
/// `~/.local/state`, plus `slopvac`.
pub fn state_dir() -> Result<PathBuf> {
    state_dir_with(&process_env, &process_home)
}

/// [`state_dir`] reading `XDG_STATE_HOME` and `HOME` through `lookup`, and asking
/// `home_fallback` when `HOME` is unset or empty.
pub fn state_dir_with(lookup: Lookup<'_>, home_fallback: HomeFallback<'_>) -> Result<PathBuf> {
    resolve(DirKind::State, lookup, home_fallback)
}

/// Which per-user directory to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirKind {
    /// Data that can be rebuilt.
    Cache,
    /// User configuration.
    Config,
    /// Durable data, such as the plugin store.
    Data,
    /// Durable state, such as the trust store.
    State,
}

impl DirKind {
    /// The XDG variable and the home-relative default for this kind.
    const fn xdg(self) -> (&'static str, &'static str) {
        match self {
            Self::Cache => ("XDG_CACHE_HOME", ".cache"),
            Self::Config => ("XDG_CONFIG_HOME", ".config"),
            Self::Data => ("XDG_DATA_HOME", ".local/share"),
            Self::State => ("XDG_STATE_HOME", ".local/state"),
        }
    }
}

/// A resolved per-user directory and how it was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedDir {
    /// The directory.
    pub path: PathBuf,
    /// The environment variable that chose it, or `default` for the home-relative location.
    pub source: &'static str,
}

/// Resolve `kind` reading the environment through `lookup`, and asking `home_fallback` when
/// `HOME` is unset or empty. Only the cache directory honours an override variable
/// (`SLOPVAC_CACHE_DIR`). An XDG value counts only when it is an absolute path.
pub fn resolve_dir_with(
    kind: DirKind,
    lookup: Lookup<'_>,
    home_fallback: HomeFallback<'_>,
) -> Result<ResolvedDir> {
    if kind == DirKind::Cache
        && let Some(dir) = value(lookup, CACHE_DIR_VAR)?.filter(|v| !v.is_empty())
    {
        return Ok(ResolvedDir {
            path: PathBuf::from(dir),
            source: CACHE_DIR_VAR,
        });
    }
    let (var, default) = kind.xdg();
    match value(lookup, var)?
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        Some(dir) => Ok(ResolvedDir {
            path: dir.join(APP),
            source: var,
        }),
        None => Ok(ResolvedDir {
            path: home(lookup, home_fallback)?.join(default).join(APP),
            source: "default",
        }),
    }
}

fn resolve(kind: DirKind, lookup: Lookup<'_>, home_fallback: HomeFallback<'_>) -> Result<PathBuf> {
    resolve_dir_with(kind, lookup, home_fallback).map(|d| d.path)
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

    #[rstest::rstest]
    #[case::data_xdg(DirKind::Data, &[("XDG_DATA_HOME", "/xd"), ("HOME", "/h")], "/xd/slopvac", "XDG_DATA_HOME")]
    #[case::data_default(DirKind::Data, &[("HOME", "/h")], "/h/.local/share/slopvac", "default")]
    #[case::data_relative_xdg(DirKind::Data, &[("XDG_DATA_HOME", "rel"), ("HOME", "/h")], "/h/.local/share/slopvac", "default")]
    #[case::data_ignores_cache_override(DirKind::Data, &[(CACHE_DIR_VAR, "/c"), ("HOME", "/h")], "/h/.local/share/slopvac", "default")]
    #[case::state_xdg(DirKind::State, &[("XDG_STATE_HOME", "/xs"), ("HOME", "/h")], "/xs/slopvac", "XDG_STATE_HOME")]
    #[case::state_default(DirKind::State, &[("HOME", "/h")], "/h/.local/state/slopvac", "default")]
    #[case::state_empty_xdg(DirKind::State, &[("XDG_STATE_HOME", ""), ("HOME", "/h")], "/h/.local/state/slopvac", "default")]
    #[case::cache_override(DirKind::Cache, &[(CACHE_DIR_VAR, "/c"), ("XDG_CACHE_HOME", "/x")], "/c", "SLOPVAC_CACHE_DIR")]
    #[case::cache_xdg(DirKind::Cache, &[("XDG_CACHE_HOME", "/x")], "/x/slopvac", "XDG_CACHE_HOME")]
    #[case::config_default(DirKind::Config, &[("HOME", "/h")], "/h/.config/slopvac", "default")]
    fn resolve_dir_names_the_path_and_its_source(
        #[case] kind: DirKind,
        #[case] vars: &[(&str, &str)],
        #[case] path: &str,
        #[case] source: &str,
    ) {
        let got = resolve_dir_with(kind, &env(vars), &no_home).unwrap();
        assert_eq!(got.path, Path::new(path));
        assert_eq!(got.source, source);
    }

    #[test]
    fn data_and_state_wrappers_follow_the_xdg_rules() {
        let e = env(&[("XDG_DATA_HOME", "/xd"), ("HOME", "/h")]);
        assert_eq!(
            data_dir_with(&e, &no_home).unwrap(),
            Path::new("/xd/slopvac")
        );
        assert_eq!(
            state_dir_with(&e, &no_home).unwrap(),
            Path::new("/h/.local/state/slopvac")
        );
        assert!(matches!(
            state_dir_with(&env(&[]), &no_home),
            Err(Error::NoHomeDir)
        ));
    }
}
