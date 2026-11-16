//! Per-user directories.

use std::path::PathBuf;

use etcetera::BaseStrategy;

use crate::{Error, Result};

/// The environment variable that overrides the cache directory, as in 2.x.
pub const CACHE_DIR_VAR: &str = "SLOPVAC_CACHE_DIR";

const APP: &str = "slopvac";

/// The cache directory: `SLOPVAC_CACHE_DIR` when set and non-empty, else the per-user cache
/// directory that `etcetera` reports, plus `slopvac`.
pub fn cache_dir() -> Result<PathBuf> {
    cache_dir_with(&|name| std::env::var(name).ok())
}

/// [`cache_dir`] with an injectable lookup for the override variable.
///
/// Only the override reads through `lookup`. `etcetera` reads `XDG_CACHE_HOME` and the home
/// directory from the process environment itself, and offers no way to inject them.
pub fn cache_dir_with(lookup: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf> {
    if let Some(dir) = lookup(CACHE_DIR_VAR).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Ok(strategy()?.cache_dir().join(APP))
}

/// The per-user configuration directory that `etcetera` reports, plus `slopvac`.
pub fn config_dir() -> Result<PathBuf> {
    Ok(strategy()?.config_dir().join(APP))
}

fn strategy() -> Result<impl BaseStrategy> {
    etcetera::choose_base_strategy().map_err(|_| Error::NoHomeDir)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn override_wins() {
        let got = cache_dir_with(&|n| (n == CACHE_DIR_VAR).then(|| "/custom/c".to_owned()));
        assert_eq!(got.unwrap(), Path::new("/custom/c"));
    }

    #[test]
    fn empty_override_falls_back_to_the_user_cache() {
        let got = cache_dir_with(&|_| Some(String::new())).unwrap();
        assert_eq!(got.file_name().unwrap(), "slopvac");
    }

    #[test]
    fn config_dir_ends_in_the_app_name() {
        assert_eq!(config_dir().unwrap().file_name().unwrap(), "slopvac");
    }
}
