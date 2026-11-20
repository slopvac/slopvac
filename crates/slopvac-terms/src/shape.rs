//! The shape of `.slopvac/terms.toml`, as `schemas/v3/terms.schema.json`
//! defines it.
//!
//! Only `term` is required in an entry. Unknown keys are errors except keys
//! that start with `x-`, which are ignored.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::messages::TermsText;
use crate::{EntryProblem, TermsError};

/// One term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermEntry {
    /// Canonical spelling.
    pub term: String,
    /// Other accepted spellings, such as `K8s`. Non-empty and unique.
    pub variants: Vec<String>,
    /// Kebab-case label such as `product`, `language`, `protocol` or `acronym`.
    pub kind: Option<String>,
    /// Accept any casing; the term is then an allow-list entry only.
    pub ignore_case: bool,
    /// Match only the exact spelling. Other casings are different words, so
    /// ordinary words such as `Go` never produce a finding.
    pub case_sensitive: bool,
}

impl TermEntry {
    /// Check the entry against the terms schema: non-empty term, kebab-case
    /// kind, non-empty and unique variants, and at most one casing mode.
    /// `path` is used in messages only.
    ///
    /// # Errors
    /// [`TermsError::Entry`] naming the first problem.
    pub(crate) fn validate(&self, path: &str) -> Result<(), TermsError> {
        let err = |problem: EntryProblem, message: String| TermsError::Entry {
            path: path.to_owned(),
            message,
            number: None,
            problem,
        };
        if self.term.trim().is_empty() {
            return Err(err(
                EntryProblem::MissingWord,
                TermsText::TermEmpty.to_string(),
            ));
        }
        if self.ignore_case && self.case_sensitive {
            return Err(err(
                EntryProblem::ConflictingCasing,
                TermsText::ConflictingCasing {
                    term: self.term.clone(),
                }
                .to_string(),
            ));
        }
        if let Some(kind) = self.kind.as_deref().filter(|k| !is_kebab_label(k)) {
            return Err(err(
                EntryProblem::BadKind,
                TermsText::BadKind {
                    term: self.term.clone(),
                    kind: format!("{kind:?}"),
                }
                .to_string(),
            ));
        }
        let mut variants = BTreeSet::new();
        for v in &self.variants {
            if v.is_empty() {
                return Err(err(
                    EntryProblem::BadVariant,
                    TermsText::EmptyVariant {
                        term: self.term.clone(),
                    }
                    .to_string(),
                ));
            }
            if !variants.insert(v.as_str()) {
                return Err(err(
                    EntryProblem::BadVariant,
                    TermsText::RepeatedVariant {
                        term: self.term.clone(),
                        variant: v.clone(),
                    }
                    .to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// A parsed terms file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermsFile {
    /// Schema version of the file; must be 3.
    pub schema_version: u32,
    /// The terms, in file order.
    pub terms: Vec<TermEntry>,
}

/// The only `schema_version` of a terms file this crate reads.
pub const TERMS_SCHEMA_VERSION: u32 = 3;

#[derive(Deserialize)]
struct RawFile {
    schema_version: u32,
    terms: Option<Vec<RawTerm>>,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
struct RawTerm {
    term: String,
    #[serde(default)]
    variants: Vec<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    ignore_case: bool,
    #[serde(default)]
    case_sensitive: bool,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

/// Whether `kind` matches `^[a-z][a-z0-9-]*$`.
fn is_kebab_label(kind: &str) -> bool {
    let mut chars = kind.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl TermsFile {
    /// The bundled terms file.
    ///
    /// # Panics
    /// If the bundled data is malformed; the crate's tests prevent that.
    #[must_use]
    pub fn builtin() -> Self {
        Self::parse("builtin terms", include_str!("../data/builtin.toml"))
            .expect("bundled terms file is valid")
    }

    /// Parse terms TOML. `path` is used in messages only.
    ///
    /// # Errors
    /// Bad TOML or shape, wrong `schema_version`, unknown non-`x-` keys, and
    /// empty or duplicate terms.
    pub fn parse(path: &str, text: &str) -> Result<Self, TermsError> {
        let raw: RawFile = toml::from_str(text).map_err(|e| TermsError::Parse {
            path: path.to_owned(),
            message: e.to_string(),
        })?;
        if raw.schema_version != TERMS_SCHEMA_VERSION {
            return Err(TermsError::SchemaVersion {
                path: path.to_owned(),
                found: raw.schema_version.to_string(),
            });
        }
        let entry_err = |problem: EntryProblem, number: usize, message: String| TermsError::Entry {
            path: path.to_owned(),
            message,
            number: Some(number),
            problem,
        };
        let unknown = |key: &str| TermsError::UnknownKey {
            path: path.to_owned(),
            key: key.to_owned(),
        };
        if let Some(key) = raw.extra.keys().find(|k| !k.starts_with("x-")) {
            return Err(unknown(key));
        }
        let Some(raw_terms) = raw.terms else {
            return Err(TermsError::Parse {
                path: path.to_owned(),
                message: TermsText::MissingTermsArray.to_string(),
            });
        };
        let mut seen = BTreeSet::new();
        let mut terms = Vec::with_capacity(raw_terms.len());
        for (i, t) in raw_terms.into_iter().enumerate() {
            if let Some(key) = t.extra.keys().find(|k| !k.starts_with("x-")) {
                return Err(unknown(&format!("terms[{}].{key}", i + 1)));
            }
            let entry = TermEntry {
                term: t.term,
                variants: t.variants,
                kind: t.kind,
                ignore_case: t.ignore_case,
                case_sensitive: t.case_sensitive,
            };
            if entry.term.trim().is_empty() {
                return Err(entry_err(
                    EntryProblem::MissingWord,
                    i + 1,
                    TermsText::TermEmptyAt { number: i + 1 }.to_string(),
                ));
            }
            entry.validate(path).map_err(|e| e.at_entry(i + 1))?;
            if !seen.insert(entry.term.clone()) {
                return Err(entry_err(
                    EntryProblem::Duplicate,
                    i + 1,
                    TermsText::DuplicateTerm {
                        term: entry.term.clone(),
                    }
                    .to_string(),
                ));
            }
            terms.push(entry);
        }
        Ok(Self {
            schema_version: raw.schema_version,
            terms,
        })
    }

    /// Read and parse a terms file.
    ///
    /// # Errors
    /// As [`TermsFile::parse`], plus read failures.
    pub fn load(path: &std::path::Path) -> Result<Self, TermsError> {
        Self::load_labeled(path, &path.display().to_string())
    }

    /// Like [`TermsFile::load`], naming the file `shown` in errors. Pass a label that is
    /// relative to the project root, so an error never carries an absolute path.
    ///
    /// # Errors
    /// As [`TermsFile::load`].
    pub fn load_labeled(path: &std::path::Path, shown: &str) -> Result<Self, TermsError> {
        let shown = shown.to_owned();
        let text = crate::read::read_existing(path, &shown, crate::MAX_TERMS_FILE_BYTES)?;
        Self::parse(&shown, &text)
    }
}
