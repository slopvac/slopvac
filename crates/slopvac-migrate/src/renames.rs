//! The 2.x to 3.0 rename table for rules and categories.
//!
//! `tests/fixtures/generate.py` writes `data/renames.toml` from the 2.x ruleset and
//! the rule converter (docs/contract/config.md, section 5.2). A name with no entry
//! is unknown, and `migrate` stops on it.

use std::collections::{BTreeMap, BTreeSet};

use toml_edit::{DocumentMut, Item};

use crate::Error;
use crate::messages::MigrateText;

const BUNDLED: &str = include_str!("../data/renames.toml");

/// Maps 2.x rule and category names to their 3.0 qualified names.
#[derive(Debug, Clone)]
pub struct RenameTable {
    rules: BTreeMap<String, String>,
    categories: BTreeMap<String, String>,
    removed: BTreeSet<String>,
    removed_categories: BTreeSet<String>,
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
            removed: read_removed(&doc, "rules")?,
            removed_categories: read_removed(&doc, "categories")?,
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

    /// Whether this 2.x category owns no rule, so 3.0 has no category of that name.
    /// `migrate` drops every reference to it.
    #[must_use]
    pub fn is_removed_category(&self, name: &str) -> bool {
        self.removed_categories.contains(name)
    }

    /// The 3.0 name for a 2.x category, or `None` when the name is unknown.
    #[must_use]
    pub fn category(&self, name: &str) -> Option<String> {
        self.categories.get(name).cloned()
    }
}

fn read_removed(doc: &DocumentMut, key: &str) -> Result<BTreeSet<String>, Error> {
    let Some(item) = doc.get("removed") else {
        return Ok(BTreeSet::new());
    };
    let Some(array) = item.get(key) else {
        return Ok(BTreeSet::new());
    };
    array
        .as_array()
        .ok_or_else(|| {
            Error::RenameTable(
                MigrateText::RenameRemovedNotArray {
                    key: key.to_owned(),
                }
                .to_string(),
            )
        })?
        .iter()
        .map(|v| {
            v.as_str().map(str::to_owned).ok_or_else(|| {
                Error::RenameTable(
                    MigrateText::RenameRemovedNotStrings {
                        key: key.to_owned(),
                    }
                    .to_string(),
                )
            })
        })
        .collect()
}

fn read_map(doc: &DocumentMut, key: &str) -> Result<BTreeMap<String, String>, Error> {
    let table = doc.get(key).and_then(Item::as_table_like).ok_or_else(|| {
        Error::RenameTable(
            MigrateText::RenameMissingTable {
                key: key.to_owned(),
            }
            .to_string(),
        )
    })?;
    table
        .iter()
        .map(|(k, v)| {
            v.as_str()
                .map(|s| (k.to_owned(), s.to_owned()))
                .ok_or_else(|| {
                    Error::RenameTable(
                        MigrateText::RenameNotString {
                            key: key.to_owned(),
                            name: k.to_owned(),
                        }
                        .to_string(),
                    )
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy::{Legacy, PROFILES, category_of};

    #[test]
    fn bundled_table_parses() {
        let _ = RenameTable::bundled();
    }

    #[test]
    fn a_2x_rule_moves_into_the_core_pack() {
        let t = RenameTable::bundled();
        assert_eq!(
            t.rule("prose-format.no-unicode-dash").as_deref(),
            Some("core:prose-format.no-unicode-dash")
        );
        assert_eq!(
            t.category("prose-format").as_deref(),
            Some("core:prose-format")
        );
    }

    #[test]
    fn names_2x_knew_without_a_rule_file_have_entries() {
        // The generated locale rule and the directive finding keep their names in
        // the 3.0 engine; `meta` is the category of that finding.
        let t = RenameTable::bundled();
        for (old, new) in [
            ("ste-words.spelling", "core:ste-words.spelling"),
            ("meta.invalid-suppression", "core:meta.invalid-suppression"),
        ] {
            assert_eq!(t.rule(old).as_deref(), Some(new), "{old}");
        }
        assert!(t.is_removed_category("meta"));
        assert_eq!(t.category("meta"), None);
    }

    #[test]
    fn names_without_an_entry_are_unknown() {
        let t = RenameTable::bundled();
        assert_eq!(t.rule("prose-craft.no-such-rule"), None);
        assert_eq!(t.rule("prose-craft"), None);
        assert_eq!(t.category("no-such-category"), None);
        assert_eq!(t.category("prose-craft.spacing"), None);
    }

    #[test]
    fn a_value_that_is_not_a_string_is_rejected() {
        let err = RenameTable::parse("[rules]\n\"a.b\" = 1\n[categories]\n").unwrap_err();
        assert_eq!(err.code(), "migrate-rename-table");
    }

    #[test]
    fn every_name_the_legacy_data_uses_has_an_entry() {
        let t = RenameTable::bundled();
        let legacy = Legacy::bundled();
        for qid in legacy.rules.keys() {
            assert!(t.rule(qid).is_some() || t.is_removed(qid), "rule {qid}");
            assert!(t.category(category_of(qid)).is_some(), "category of {qid}");
        }
        for p in PROFILES {
            let style = legacy.style(p);
            for name in style.categories.keys() {
                assert!(t.category(name).is_some(), "{p}: category {name}");
            }
            for qid in style.rules.keys() {
                assert!(
                    t.rule(qid).is_some() || t.is_removed(qid),
                    "{p}: rule {qid}"
                );
            }
        }
    }
}
