use slopvac_config::{Code, PackFile, PresetFailure, Presets, Setting};

use crate::resolve::apply_order;
use crate::{StyleError, StyleSource};

/// A [`StyleSource`] offered to the config stack as its style layers.
///
/// `LoadedConfig::resolve` in `slopvac-config` hands over the whole `styles` list. The
/// answer is one layer per style in application order, parents first, each ancestor once,
/// so the result matches [`fn@crate::resolve`] and a value in the explain output names the
/// preset that set it. [`fn@crate::resolve`] also checks the list for cycles, malformed
/// files and conflicts; `style_layers` has no error channel, so a list that fails to load
/// contributes nothing. A style file over its size cap is the exception: [`Presets::check_styles`]
/// reports it, and the config load fails with the code `style-too-large`.
#[derive(Debug, Clone, Copy)]
pub struct StylePresets<S>(pub S);

impl<S: StyleSource> Presets for StylePresets<S> {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        let chain = apply_order(&[name.to_owned()], &self.0).ok()?;
        chain.last().map(|(_, s)| s.layer().to_vec())
    }

    fn style_layers(&self, listed: &[String]) -> Vec<Vec<Setting>> {
        apply_order(listed, &self.0)
            .map(|chain| chain.iter().map(|(_, s)| s.layer().to_vec()).collect())
            .unwrap_or_default()
    }

    fn check_styles(&self, listed: &[String]) -> Result<(), PresetFailure> {
        match apply_order(listed, &self.0) {
            Err(e @ StyleError::TooLarge { .. }) => Err(PresetFailure {
                code: Code::StyleTooLarge,
                message: e.to_string(),
            }),
            _ => Ok(()),
        }
    }

    fn genre_chain(&self, _name: &str) -> Vec<Vec<Setting>> {
        Vec::new()
    }

    fn pack_config(&self, _name: &str) -> Option<PackFile> {
        None
    }
}
