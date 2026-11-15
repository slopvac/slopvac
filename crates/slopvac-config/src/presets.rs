//! What packs supply to the layer stack: style presets, genre chains and pack configs.

use crate::source::Setting;

/// A config file that a pack ships.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFile {
    /// Path shown in provenance, `pack:<id>/<path>`.
    pub display: String,
    /// The file text.
    pub text: String,
}

/// Source of the layers that come from packs.
pub trait Presets {
    /// The settings of a style preset, in layer `style`. `None` when no active pack has it.
    fn style(&self, name: &str) -> Option<Vec<Setting>>;

    /// The settings of a genre and of the genres it extends, lowest first, in layer `genre`.
    fn genre_chain(&self, name: &str) -> Vec<Vec<Setting>>;

    /// A config that a pack ships, named by an `extends = ["<pack>:<name>"]` entry.
    fn pack_config(&self, name: &str) -> Option<PackFile>;
}

// STUB(slopvac-8l4.4.10): no pack supplies anything until the pack loader lands, so styles,
// genres and `<pack>:<name>` extends targets all resolve to nothing.
/// A [`Presets`] with no packs behind it.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPresets;

impl Presets for NoPresets {
    fn style(&self, _name: &str) -> Option<Vec<Setting>> {
        None
    }

    fn genre_chain(&self, _name: &str) -> Vec<Vec<Setting>> {
        Vec::new()
    }

    fn pack_config(&self, _name: &str) -> Option<PackFile> {
        None
    }
}
