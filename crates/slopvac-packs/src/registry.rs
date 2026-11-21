//! A registry of named items, keyed by qualified name.

use std::collections::BTreeMap;

use crate::name::{NameError, QualifiedName, is_local};
use crate::provenance::Provenance;

type Result<T> = std::result::Result<T, RegistryError>;

/// What a registry operation can refuse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// Two suppliers registered the same qualified name.
    #[error("`{name}` is provided twice: by {first} and by {second}")]
    Duplicate {
        /// The contested name.
        name: QualifiedName,
        /// The supplier that registered it first.
        first: Box<Provenance>,
        /// The supplier that tried to register it again.
        second: Box<Provenance>,
    },
    /// A bare name is claimed by more than one pack. The stable code is `pack-name-ambiguous`.
    #[error("{}", crate::messages::name_ambiguous(.name, .claims, .at.as_ref()))]
    Ambiguous {
        /// The bare name.
        name: String,
        /// Every claim, in qualified-name order.
        claims: Vec<(QualifiedName, Provenance)>,
        /// Where the bare name was written, when the caller knows.
        at: Option<Reference>,
    },
    /// No item has this name.
    #[error("no item named `{}`", slopvac_sanitize::line(.0))]
    Unknown(String),
    /// The text is neither a bare name nor a qualified name.
    #[error(transparent)]
    Name(#[from] NameError),
}

/// Where a name was written: a file and a one-based line and column.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Reference {
    /// The file, as the caller names it.
    pub file: String,
    /// One-based line.
    pub line: u32,
    /// One-based column.
    pub column: u32,
}

impl Reference {
    /// A reference at `file`, `line` and `column`.
    #[must_use]
    pub fn new(file: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            file: file.into(),
            line,
            column,
        }
    }
}

impl RegistryError {
    /// The stable code of this error, for the variants that have one.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::Ambiguous { .. } => Some(crate::messages::code_of!(NameAmbiguous {
                at,
                name,
                claims
            })),
            _ => None,
        }
    }
}

/// An item with the provenance that supplied it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry<T> {
    /// The registered item.
    pub value: T,
    /// Who supplied it.
    pub provenance: Provenance,
}

/// Named items from several packs. A qualified name is unique; a bare name resolves only when
/// one pack claims it.
#[derive(Debug, Clone)]
pub struct Registry<T> {
    entries: BTreeMap<QualifiedName, Entry<T>>,
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}

impl<T> Registry<T> {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `value` under `name`. A second registration of the same name is an error that
    /// names both suppliers, and the registry keeps the first.
    pub fn insert(&mut self, name: QualifiedName, value: T, provenance: Provenance) -> Result<()> {
        if let Some(existing) = self.entries.get(&name) {
            return Err(RegistryError::Duplicate {
                name,
                first: Box::new(existing.provenance.clone()),
                second: Box::new(provenance),
            });
        }
        self.entries.insert(name, Entry { value, provenance });
        Ok(())
    }

    /// The entry for an exact qualified name.
    #[must_use]
    pub fn get(&self, name: &QualifiedName) -> Option<&Entry<T>> {
        self.entries.get(name)
    }

    /// Resolve user text: a qualified name (`pack:name`) looks up exactly, and a bare name must
    /// be claimed by one pack.
    pub fn resolve(&self, text: &str) -> Result<&Entry<T>> {
        self.resolve_inner(text, None)
    }

    /// Like [`Registry::resolve`], for a name written at `at`: an ambiguity error points there.
    pub fn resolve_at(&self, text: &str, at: &Reference) -> Result<&Entry<T>> {
        self.resolve_inner(text, Some(at))
    }

    fn resolve_inner(&self, text: &str, at: Option<&Reference>) -> Result<&Entry<T>> {
        if text.contains(':') {
            let name: QualifiedName = text.parse()?;
            return self
                .entries
                .get(&name)
                .ok_or_else(|| RegistryError::Unknown(text.to_owned()));
        }
        if !is_local(text) {
            return Err(NameError::Local(text.to_owned()).into());
        }
        let mut claims = self.entries.iter().filter(|(q, _)| q.name() == text);
        match (claims.next(), claims.next()) {
            (None, _) => Err(RegistryError::Unknown(text.to_owned())),
            (Some((_, entry)), None) => Ok(entry),
            (Some(first), Some(second)) => Err(RegistryError::Ambiguous {
                name: text.to_owned(),
                claims: [first, second]
                    .into_iter()
                    .chain(claims)
                    .map(|(q, e)| (q.clone(), e.provenance.clone()))
                    .collect(),
                at: at.cloned(),
            }),
        }
    }

    /// Every entry, in qualified-name order.
    pub fn iter(&self) -> impl Iterator<Item = (&QualifiedName, &Entry<T>)> {
        self.entries.iter()
    }

    /// How many items are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
