//! The shape of `.slopvac/terms.toml`, as `schemas/v3/terms.schema.json`
//! defines it.
//!
//! Only `term` is required in an entry. Unknown keys are errors except keys
//! that start with `x-`, which are ignored.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::TermsError;

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

/// A parsed terms file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermsFile {
    /// Schema version of the file; must be 3.
    pub schema_version: u32,
    /// The terms, in file order.
    pub terms: Vec<TermEntry>,
}

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
        if raw.schema_version != 3 {
            return Err(TermsError::SchemaVersion {
                path: path.to_owned(),
                found: raw.schema_version,
            });
        }
        let entry_err = |message: String| TermsError::Entry {
            path: path.to_owned(),
            message,
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
                message: "missing `terms` array; use `terms = []` for an empty list".to_owned(),
            });
        };
        let mut seen = BTreeSet::new();
        let mut terms = Vec::with_capacity(raw_terms.len());
        for (i, t) in raw_terms.into_iter().enumerate() {
            if let Some(key) = t.extra.keys().find(|k| !k.starts_with("x-")) {
                return Err(unknown(&format!("terms[{}].{key}", i + 1)));
            }
            if t.term.trim().is_empty() {
                return Err(entry_err(format!("term {} is empty", i + 1)));
            }
            if t.ignore_case && t.case_sensitive {
                return Err(entry_err(format!(
                    "term `{}` sets both ignore_case and case_sensitive",
                    t.term
                )));
            }
            if let Some(kind) = t.kind.as_deref().filter(|k| !is_kebab_label(k)) {
                return Err(entry_err(format!(
                    "term `{}` has kind {kind:?}; use a kebab-case label such as `product`",
                    t.term
                )));
            }
            let mut variants = BTreeSet::new();
            for v in &t.variants {
                if v.is_empty() {
                    return Err(entry_err(format!("term `{}` has an empty variant", t.term)));
                }
                if !variants.insert(v.as_str()) {
                    return Err(entry_err(format!(
                        "term `{}` lists variant `{v}` twice",
                        t.term
                    )));
                }
            }
            if !seen.insert(t.term.clone()) {
                return Err(entry_err(format!("duplicate term `{}`", t.term)));
            }
            terms.push(TermEntry {
                term: t.term,
                variants: t.variants,
                kind: t.kind,
                ignore_case: t.ignore_case,
                case_sensitive: t.case_sensitive,
            });
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
        let shown = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|e| TermsError::Io {
            path: shown.clone(),
            message: e.to_string(),
        })?;
        Self::parse(&shown, &text)
    }
}
