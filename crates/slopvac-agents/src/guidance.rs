//! The `[guidance]` table of `slopvac.toml`, as `prime` and `setup` read it.
//!
//! Loading goes through `slopvac-config`, so layering, `extends` and schema validation
//! behave as they do for `lint`. A bad value is a `config-*` error with exit code 2.

use std::path::Path;

use slopvac_config::{ConfigError, Discovery, LoadedConfig, NoPresets};

use crate::error::{AgentsError, ConfigFailure};
use crate::prime::Level;

/// The guidance settings that `prime` and `setup` act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guidance {
    /// `guidance.level`: detail of the rendered `prime` topics.
    pub level: Level,
    /// `guidance.write` is not `"off"`.
    pub write: bool,
    /// `guidance.review` is not `"off"`.
    pub review: bool,
}

impl Default for Guidance {
    fn default() -> Self {
        Self {
            level: Level::Default,
            write: true,
            review: true,
        }
    }
}

impl Guidance {
    /// Read the guidance for the project at `root`.
    ///
    /// `explicit` is a `--config` path and wins over discovery. Without it, the nearest
    /// `slopvac.toml` from `root` upward applies. With no config the schema defaults hold.
    pub fn load(root: &Path, explicit: Option<&Path>) -> Result<Self, AgentsError> {
        let discovery = Discovery::new(&NoPresets);
        let config = match explicit {
            Some(path) => Some(discovery.explicit(path)),
            None => discovery.nearest(root).transpose(),
        };
        match config {
            None => Ok(Self::default()),
            Some(Ok(config)) => Ok(Self::from_config(&config)),
            Some(Err(e)) => Err(config_error(e)),
        }
    }

    /// Whether setup installs the skill: any guidance on. With write and review both off it
    /// installs none, as 2.x does.
    #[must_use]
    pub fn skill_wanted(&self) -> bool {
        self.write || self.review
    }

    fn from_config(config: &LoadedConfig) -> Self {
        let text = |key: &str| {
            config
                .project_value(&["guidance", key])
                .and_then(|v| v.as_str().map(str::to_owned))
        };
        Self {
            level: match text("level").as_deref() {
                Some("brief") => Level::Minimal,
                Some("full") => Level::Full,
                _ => Level::Default,
            },
            write: text("write").as_deref() != Some("off"),
            review: text("review").as_deref() != Some("off"),
        }
    }
}

fn config_error(e: ConfigError) -> AgentsError {
    AgentsError::Config {
        code: e.code().as_str(),
        source: ConfigFailure::new(e),
    }
}
