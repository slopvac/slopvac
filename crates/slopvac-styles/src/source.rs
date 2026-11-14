use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::{Style, StyleError};

/// Where styles come from. Packs implement this once they exist.
pub trait StyleSource {
    /// Load one style by name.
    ///
    /// # Errors
    /// [`StyleError::NotFound`] when absent, parse errors when malformed.
    fn load(&self, name: &str) -> Result<Style, StyleError>;
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Styles held in memory as TOML text. For tests and embedded presets.
#[derive(Debug, Default, Clone)]
pub struct MapSource(BTreeMap<String, String>);

impl MapSource {
    /// An empty source.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a style by name.
    #[must_use]
    pub fn with(mut self, name: &str, toml: &str) -> Self {
        self.0.insert(name.to_owned(), toml.to_owned());
        self
    }
}

impl StyleSource for MapSource {
    fn load(&self, name: &str) -> Result<Style, StyleError> {
        let text = self
            .0
            .get(name)
            .ok_or_else(|| StyleError::NotFound(name.to_owned()))?;
        Style::parse(name, text)
    }
}

/// The presets that ship inside the binary.
#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinSource;

pub(crate) const BUILTIN: &[(&str, &str)] = &[
    ("ste", include_str!("../presets/ste.toml")),
    ("google", include_str!("../presets/google.toml")),
];

impl BuiltinSource {
    /// Names of the bundled presets.
    pub fn names() -> impl Iterator<Item = &'static str> {
        BUILTIN.iter().map(|(n, _)| *n)
    }
}

impl StyleSource for BuiltinSource {
    fn load(&self, name: &str) -> Result<Style, StyleError> {
        let (_, text) = BUILTIN
            .iter()
            .find(|(n, _)| *n == name)
            .ok_or_else(|| StyleError::NotFound(name.to_owned()))?;
        Style::parse(name, text)
    }
}

/// Styles in a directory as `<name>.toml`.
#[derive(Debug, Clone)]
pub struct DirSource(PathBuf);

impl DirSource {
    /// A source rooted at `dir`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self(dir.into())
    }
}

impl StyleSource for DirSource {
    fn load(&self, name: &str) -> Result<Style, StyleError> {
        if !valid_name(name) {
            return Err(StyleError::InvalidName(name.to_owned()));
        }
        let path = self.0.join(format!("{name}.toml"));
        match std::fs::read_to_string(&path) {
            Ok(text) => Style::parse(name, &text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(StyleError::NotFound(name.to_owned()))
            }
            Err(e) => Err(StyleError::Io {
                style: name.to_owned(),
                message: e.to_string(),
            }),
        }
    }
}
