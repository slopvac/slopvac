//! The config resolver that `slopvac migrate` lints each side with: the same path from a
//! config to engine settings that `slopvac lint` takes.

use std::path::Path;

use slopvac_config::LoadedConfig;
use slopvac_engine::{Engine, Input, Kind};
use slopvac_migrate::{Configure, DocumentSettings, Error, SettingsOf};

use crate::engine::{block_errors, config_error, file_settings, rel_text};
use crate::errors::CliError;
use crate::lint_settings::{self, DocumentBlocks, DocumentLayers, Overrides};
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
                lint_settings::presets(),
            )
            .map_err(|e| lint_error(config_error(&e, dir)))?,
            None => LoadedConfig::without_files(dir.to_owned()),
        };
        // Unknown rule ids only warn in `slopvac lint`, so they do not stop the comparison.
        lint_settings::validate(&loaded, &self.rules).map_err(lint_error)?;
        // The genre of a document follows its content, and the settings follow the genre, so
        // an engine over the settings before any genre layer reads each document first, as a
        // lint run does.
        let probe = Catalog::load().map_err(lint_error).and_then(|catalog| {
            let (settings, _) = file_settings(
                &catalog,
                &loaded,
                Path::new(""),
                &Overrides::default(),
                &DocumentLayers::default(),
            )
            .map_err(lint_error)?;
            Engine::new(catalog.into_sets(), settings).map_err(|e| Error::Lint(e.to_string()))
        })?;
        Ok(Box::new(move |rel, text| {
            let shown = rel.display().to_string();
            let blocks = DocumentBlocks::read(text, &shown, Some(&loaded));
            // A block or directive that a lint run reports as an error stops the comparison,
            // as a document that was not checked in full does.
            if let Some(failure) = block_errors(&shown, text, &blocks).into_iter().next() {
                return Err(Error::Lint(format!("{}: {}", failure.code, failure.raw)));
            }
            let relative = rel_text(rel);
            let mut input = Input::new(&shown, text)
                .with_kind(kind_of(rel))
                .with_relative_path(&relative);
            let declared = blocks.declared_genre();
            if let Some(genre) = &declared {
                input = input.with_declared_genre(genre);
            }
            let detected = probe
                .preparse(&input)
                .ok()
                .and_then(|pre| probe.detect_genre(&input, &pre).ok().flatten())
                .map(|genre| genre.id.as_str().to_owned());
            let layers = DocumentLayers::from_blocks(&blocks).with_detected_genre(detected);
            file_settings(&self.rules, &loaded, rel, &Overrides::default(), &layers)
                .map(|(settings, _)| DocumentSettings::new(settings).with_declared_genre(declared))
                .map_err(lint_error)
        }))
    }
}

/// The engine kind of a document by its extension, as discovery in `slopvac lint` reads it.
fn kind_of(rel: &Path) -> Kind {
    match rel.extension().and_then(|e| e.to_str()) {
        Some("mdx") => Kind::Mdx,
        Some("rst") => Kind::Rst,
        _ => Kind::Markdown,
    }
}
