//! A [`Configure`] double for tests that lint projects without a config.

use std::path::Path;

use slopvac_engine::Settings;
use slopvac_migrate::{Configure, DocumentSettings, Error, SettingsOf};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString};

/// Gives every document the core defaults: profile `normal` and the default genre
/// `reference`. A config it is handed fails the load, since these tests have none.
pub(crate) struct NoConfig;

impl Configure for NoConfig {
    fn load<'a>(&'a self, _dir: &Path, config: Option<&str>) -> Result<SettingsOf<'a>, Error> {
        if config.is_some() {
            return Err(Error::Lint("this double reads no config".into()));
        }
        let failed = |e: &dyn std::fmt::Display| Error::Lint(e.to_string());
        Ok(Box::new(move |_, _| {
            Ok(DocumentSettings::new(Settings::new(
                NonEmptyString::new("normal").map_err(|e| failed(&e))?,
                GenreResult {
                    id: GenreId::new("reference").map_err(|e| failed(&e))?,
                    source: GenreSource::Default,
                    reasons: Vec::new(),
                },
            )))
        }))
    }
}
