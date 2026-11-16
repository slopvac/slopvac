use slopvac_config::{PackFile, Presets, Setting};

use crate::StyleSource;
use crate::resolve::apply_order;

/// A [`StyleSource`] offered to the config stack as its style layer.
///
/// `LoadedConfig::resolve` in `slopvac-config` asks for each name in `styles`. The answer
/// is the settings of the style and of everything it extends, parents first, so a value in
/// the explain output names the preset that set it. [`fn`] checks the list for
/// cycles, malformed files and conflicts; this adapter has no error channel, so a style
/// that fails to load contributes nothing.
#[derive(Debug, Clone, Copy)]
pub struct StylePresets<S>(pub S);

impl<S: StyleSource> Presets for StylePresets<S> {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        let chain = apply_order(&[name.to_owned()], &self.0).ok()?;
        Some(
            chain
                .iter()
                .flat_map(|(_, s)| s.layer().iter().cloned())
                .collect(),
        )
    }

    fn genre_chain(&self, _name: &str) -> Vec<Vec<Setting>> {
        Vec::new()
    }

    fn pack_config(&self, _name: &str) -> Option<PackFile> {
        None
    }
}
