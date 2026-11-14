//! The 2.x to 3.0 rename table for rules and categories.

use std::collections::BTreeMap;

use toml_edit::{DocumentMut, Item};

use crate::Error;

const BUNDLED: &str = include_str!("../data/renames.toml");

/// Maps 2.x rule and category names to their 3.0 qualified names.
#[derive(Debug, Clone)]
pub struct RenameTable {
    default_pack: String,
    rules: BTreeMap<String, String>,
    categories: BTreeMap<String, String>,
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

    /// Parse a rename table from TOML text.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let doc: DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| Error::RenameTable(e.to_string()))?;
        let default_pack = doc
            .get("default_pack")
            .and_then(Item::as_str)
            .ok_or_else(|| Error::RenameTable("missing default_pack".into()))?
            .to_owned();
        Ok(Self {
            default_pack,
            rules: read_map(&doc, "rules"),
            categories: read_map(&doc, "categories"),
        })
    }

    /// The 3.0 id for a 2.x rule id, or `None` when the name is unknown.
    #[must_use]
    pub fn rule(&self, name: &str) -> Option<String> {
        if let Some(mapped) = self.rules.get(name) {
            return Some(mapped.clone());
        }
        is_rule_id(name).then(|| format!("{}:{name}", self.default_pack))
    }

    /// The 3.0 name for a 2.x category, or `None` when the name is unknown.
    #[must_use]
    pub fn category(&self, name: &str) -> Option<String> {
        if let Some(mapped) = self.categories.get(name) {
            return Some(mapped.clone());
        }
        is_slug(name).then(|| format!("{}:{name}", self.default_pack))
    }
}

fn read_map(doc: &DocumentMut, key: &str) -> BTreeMap<String, String> {
    doc.get(key)
        .and_then(Item::as_table)
        .map(|t| {
            t.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.to_owned(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

fn is_slug(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_rule_id(s: &str) -> bool {
    s.split_once('.')
        .is_some_and(|(cat, rule)| is_slug(cat) && is_slug(rule))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_table_parses() {
        let _ = RenameTable::bundled();
    }

    #[test]
    fn rule_gets_default_pack() {
        let t = RenameTable::bundled();
        assert_eq!(
            t.rule("prose-format.no-unicode-dash").as_deref(),
            Some("core:prose-format.no-unicode-dash")
        );
    }

    #[test]
    fn explicit_entry_wins() {
        let t = RenameTable::parse("default_pack = \"core\"\n[rules]\n\"a.b\" = \"other:a.c\"\n")
            .unwrap();
        assert_eq!(t.rule("a.b").as_deref(), Some("other:a.c"));
    }

    #[test]
    fn malformed_names_are_unknown() {
        let t = RenameTable::bundled();
        assert_eq!(t.rule("Bad Name"), None);
        assert_eq!(t.rule("nodot"), None);
        assert_eq!(t.category("Bad.Name"), None);
    }
}
