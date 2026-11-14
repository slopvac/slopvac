//! Draft shape of `.slopvac/terms.toml`.
//!
//! STUB(slopvac-8l4.8.5): the real shape is `schemas/v3/terms.schema.json`
//! (slopvac-8l4.2.4). Each term needs only `term`; `variants`, `kind` and
//! `ignore_case` are optional.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::TermsError;

/// One term.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TermEntry {
    /// Canonical spelling.
    pub term: String,
    /// Other accepted spellings, such as `K8s`.
    #[serde(default)]
    pub variants: Vec<String>,
    /// Free-form category: `product`, `language`, `protocol`, `acronym`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Accept any casing; the term is then an allow-list entry only.
    #[serde(default)]
    pub ignore_case: bool,
    /// Match only the exact spelling. Other casings are different words, so
    /// ordinary words such as `Go` never produce a finding.
    #[serde(default)]
    pub case_sensitive: bool,
}

/// A parsed terms file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TermsFile {
    /// Schema version of the file; must be 3.
    pub schema_version: u32,
    /// The terms, in file order.
    #[serde(default)]
    pub terms: Vec<TermEntry>,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
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
        let file: Self = toml::from_str(text).map_err(|e| TermsError::Parse {
            path: path.to_owned(),
            message: e.to_string(),
        })?;
        if file.schema_version != 3 {
            return Err(TermsError::SchemaVersion {
                path: path.to_owned(),
                found: file.schema_version,
            });
        }
        if let Some(key) = file.extra.keys().find(|k| !k.starts_with("x-")) {
            return Err(TermsError::UnknownKey {
                path: path.to_owned(),
                key: key.clone(),
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for (i, t) in file.terms.iter().enumerate() {
            if t.ignore_case && t.case_sensitive {
                return Err(TermsError::Entry {
                    path: path.to_owned(),
                    message: format!("term `{}` sets both ignore_case and case_sensitive", t.term),
                });
            }
            if t.term.trim().is_empty() {
                return Err(TermsError::Entry {
                    path: path.to_owned(),
                    message: format!("term {} is empty", i + 1),
                });
            }
            if !seen.insert(t.term.clone()) {
                return Err(TermsError::Entry {
                    path: path.to_owned(),
                    message: format!("duplicate term `{}`", t.term),
                });
            }
        }
        Ok(file)
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
