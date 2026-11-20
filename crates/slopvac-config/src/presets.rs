//! What packs supply to the layer stack: style presets, genre chains and pack configs.

use crate::source::Setting;

/// A config file that a pack ships.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFile {
    /// Path shown in provenance, `plugin:<id>/<path>`.
    pub display: String,
    /// The file text.
    pub text: String,
}

/// A failure that a provider reports for the `styles` list, such as a file over its size cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetFailure {
    /// The stable code of the failure.
    pub code: crate::Code,
    /// One-line statement of the problem.
    pub message: String,
}

/// Source of the layers that come from packs.
pub trait Presets {
    /// The settings of a style preset, in layer `style`. `None` when no active pack has it.
    fn style(&self, name: &str) -> Option<Vec<Setting>>;

    /// The style layers for the `styles` list, lowest first: one entry per layer. The default
    /// is one layer per listed name that [`Presets::style`] knows. A provider whose styles
    /// extend other styles overrides it, so each style keeps its own layer and a shared
    /// ancestor applies once.
    fn style_layers(&self, listed: &[String]) -> Vec<Vec<Setting>> {
        listed.iter().filter_map(|n| self.style(n)).collect()
    }

    /// Check that every style in the `styles` list can be loaded, for the failures that must
    /// stop a run. [`Presets::style_layers`] has no error channel, so a provider reports such a
    /// failure here and the loader rejects the config at the `styles` key. The default accepts
    /// every list.
    ///
    /// # Errors
    /// The failure to report, such as a style file over its size cap.
    fn check_styles(&self, _listed: &[String]) -> Result<(), PresetFailure> {
        Ok(())
    }

    /// The settings of a genre and of the genres it extends, lowest first, in layer `genre`.
    fn genre_chain(&self, name: &str) -> Vec<Vec<Setting>>;

    /// A config that a pack ships, named by an `extends = ["<plugin>:<name>"]` entry.
    fn pack_config(&self, name: &str) -> Option<PackFile>;
}
