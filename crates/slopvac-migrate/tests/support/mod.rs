//! A [`Configure`] double for tests that lint projects without a config.

use std::path::Path;

use slopvac_engine::Settings;
use slopvac_migrate::{Configure, Error, SettingsOf};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString};

/// Gives every document the core defaults: profile `normal` and the default genre
/// `reference`. A config it is handed fails the load, since these tests have none.
pub(crate) struct NoConfig;

impl Configure for NoConfig {
    fn load<'a>(&'a self, _dir: &Path, config: Option<&str>) -> Result<SettingsOf<'a>, Error> {
        if config.is_some() {
            return Err(Error::Lint("this double reads no config".into()));
        }
        Ok(Box::new(|_| {
            Ok(Settings::new(
                NonEmptyString::new("normal").expect("a profile name"),
                GenreResult {
                    id: GenreId::new("reference").expect("a genre id"),
                    source: GenreSource::Default,
                    reasons: Vec::new(),
                },
            ))
        }))
    }
}
