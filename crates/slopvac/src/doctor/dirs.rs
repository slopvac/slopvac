//! Per-user directories, resolved by `slopvac-fs` so that doctor reports the locations the plugin
//! store and the trust store use.

use std::env::VarError;

use slopvac_fs::{DirKind, resolve_dir_with};

use super::{Dirs, Located};
use crate::env::Env;

/// The cache, data and state directories from the shared resolver.
#[derive(Debug, Clone, Copy, Default)]
pub struct SharedDirs;

impl SharedDirs {
    fn locate(kind: DirKind, env: &dyn Env) -> Located {
        let lookup = |name: &str| env.var(name).ok_or(VarError::NotPresent);
        // Doctor reads the injected environment only, so a missing `HOME` stays unresolved.
        resolve_dir_with(kind, &lookup, &|| None)
            .map_or_else(|_| Located::unresolved(), |d| Located::at(d.path, d.source))
    }
}

impl Dirs for SharedDirs {
    fn cache(&self, env: &dyn Env) -> Located {
        Self::locate(DirKind::Cache, env)
    }

    fn data(&self, env: &dyn Env) -> Located {
        Self::locate(DirKind::Data, env)
    }

    fn state(&self, env: &dyn Env) -> Located {
        Self::locate(DirKind::State, env)
    }
}
