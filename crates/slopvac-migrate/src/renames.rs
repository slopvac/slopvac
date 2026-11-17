//! The 2.x to 3.0 rename table for rules and categories.
//!
//! `tests/fixtures/generate.py` writes `data/renames.toml` from the 2.x ruleset and
//! the rule converter (docs/contract/config.md, section 5.2). A name with no entry
//! is unknown, and `migrate` stops on it.

use std::collections::{BTreeMap, BTreeSet};

use toml_edit::{DocumentMut, Item};

use crate::Error;

const BUNDLED: &str = include_str!("../data/renames.toml");

/// Maps 2.x rule and category names to their 3.0 qualified names.
#[derive(Debug, Clone)]
pub struct RenameTable {
    rules: BTreeMap<String, String>,
    categories: BTreeMap<String, String>,
    removed: BTreeSet<String>,
}

impl RenameTable {
    /// The table that ships in the binary.
    ///
    /// # Panics
    /// Never: a unit test checks the bundled file.
    #[must_use]
    pub fn bundled() -> Self {
        Self::parse(BUNDLED).expect("bundled rename table parses")
    }

    /// Parse a rename table from TOML text: a `[rules]` and a `[categories]` table
    /// of 2.x names to 3.0 qualified names.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let doc: DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| Error::RenameTable(e.to_string()))?;
        Ok(Self {
            rules: read_map(&doc, "rules")?,
            categories: read_map(&doc, "categories")?,
            removed: read_removed(&doc)?,
        })
    }

    /// The 3.0 id for a 2.x rule id, or `None` when the name is unknown.
    #[must_use]
    pub fn rule(&self, name: &str) -> Option<String> {
        self.rules.get(name).cloned()
    }

    /// Whether 3.0 removed this 2.x rule. `migrate` drops every reference to it.
    #[must_use]
    pub fn is_removed(&self, name: &str) -> bool {
        self.removed.contains(name)
    }

    /// The 3.0 name for a 2.x category, or `None` when the name is unknown.
    #[must_use]
    pub fn category(&self, name: &str) -> Option<String> {
        self.categories.get(name).cloned()
    }
}

fn read_removed(doc: &DocumentMut) -> Result<BTreeSet<String>, Error> {
    let Some(item) = doc.get("removed") else {
        return Ok(BTreeSet::new());
    };
    item.get("rules")
        .and_then(Item::as_array)
        .ok_or_else(|| Error::RenameTable("removed.rules: expected an array".into()))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| Error::RenameTable("removed.rules: expected strings".into()))
        })
        .collect()
}

fn read_map(doc: &DocumentMut, key: &str) -> Result<BTreeMap<String, String>, Error> {
    let table = doc
        .get(key)
        .and_then(Item::as_table_like)
        .ok_or_else(|| Error::RenameTable(format!("missing [{key}] table")))?;
    table
        .iter()
        .map(|(k, v)| {
            v.as_str()
                .map(|s| (k.to_owned(), s.to_owned()))
                .ok_or_else(|| Error::RenameTable(format!("{key}.{k}: expected a string")))
        })
        .collect()
}
