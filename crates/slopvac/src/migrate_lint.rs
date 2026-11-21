//! The config resolver that `slopvac migrate` lints each side with: the same path from a
//! config to engine settings that `slopvac lint` takes.

use std::path::Path;

use slopvac_config::LoadedConfig;
use slopvac_migrate::{Configure, Error, SettingsOf};
use slopvac_styles::{BuiltinSource, StylePresets};

use crate::engine::{config_error, file_settings};
use crate::errors::CliError;
use crate::lint_settings::{self, Overrides};
use crate::rule_catalog::Catalog;

/// The file name of a 3.0 config.
const CONFIG_NAME: &str = "slopvac.toml";

/// Resolves each config the way `slopvac lint` does, with no command-line overrides.
#[derive(Debug)]
pub struct MigrateConfigure {
    rules: Catalog,
}

impl MigrateConfigure {
    /// A resolver over the shipped rules.
    ///
    /// # Errors
    /// `rules-unavailable` when the shipped rules do not load.
    pub fn load() -> Result<Self, CliError> {
        Ok(Self {
            rules: Catalog::load()?,
        })
    }
}

/// The failure as migrate reports it: the unescaped text, which migrate escapes once.
fn lint_error(err: CliError) -> Error {
    Error::Lint(err.report_message.unwrap_or(err.message))
}

impl Configure for MigrateConfigure {
    fn load<'a>(&'a self, dir: &Path, config: Option<&str>) -> Result<SettingsOf<'a>, Error> {
        let loaded = match config {
            Some(text) => slopvac_config::load_text(
                &dir.join(CONFIG_NAME),
                CONFIG_NAME,
                text.to_owned(),
                &StylePresets(BuiltinSource),
            )
            .map_err(|e| lint_error(config_error(&e, dir)))?,
            None => LoadedConfig::without_files(dir.to_owned()),
        };
        // Unknown rule ids only warn in `slopvac lint`, so they do not stop the comparison.
        lint_settings::validate(&loaded, &self.rules).map_err(lint_error)?;
        Ok(Box::new(move |rel| {
            file_settings(&self.rules, &loaded, rel, &Overrides::default())
                .map(|(settings, _)| settings)
                .map_err(lint_error)
        }))
    }
}
