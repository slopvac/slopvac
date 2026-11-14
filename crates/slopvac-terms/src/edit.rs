//! Comment-preserving edits to a terms file.

use std::path::Path;

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

use crate::{TermEntry, TermsError};

/// A terms file open for editing. Comments and layout survive an edit.
#[derive(Debug)]
pub struct TermsDoc {
    doc: DocumentMut,
}

fn err(path: &str, message: impl Into<String>) -> TermsError {
    TermsError::Entry {
        path: path.to_owned(),
        message: message.into(),
    }
}

impl Default for TermsDoc {
    fn default() -> Self {
        let mut doc = DocumentMut::new();
        doc["schema_version"] = value(3);
        Self { doc }
    }
}

impl TermsDoc {
    /// Open a file, or start a new document when the file does not exist.
    ///
    /// # Errors
    /// Read or parse failures.
    pub fn open(path: &Path) -> Result<Self, TermsError> {
        let shown = path.display().to_string();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                crate::TermsFile::parse(&shown, &text)?;
                let doc = text.parse::<DocumentMut>().map_err(|e| TermsError::Parse {
                    path: shown,
                    message: e.to_string(),
                })?;
                Ok(Self { doc })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(TermsError::Io {
                path: shown,
                message: e.to_string(),
            }),
        }
    }

    /// Write the document, creating parent directories.
    ///
    /// # Errors
    /// Write failures.
    pub fn save(&self, path: &Path) -> Result<(), TermsError> {
        let io = |e: std::io::Error| TermsError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        std::fs::write(path, self.doc.to_string()).map_err(io)
    }

    /// Render the document.
    #[must_use]
    pub fn render(&self) -> String {
        self.doc.to_string()
    }

    fn tables(&mut self) -> &mut ArrayOfTables {
        if !self.doc.contains_key("terms") {
            self.doc["terms"] = Item::ArrayOfTables(ArrayOfTables::new());
        }
        self.doc["terms"]
            .as_array_of_tables_mut()
            .expect("`terms` is an array of tables; open() validated the file")
    }

    /// Whether a term with this canonical spelling exists.
    #[must_use]
    pub fn contains(&self, term: &str) -> bool {
        self.doc
            .get("terms")
            .and_then(Item::as_array_of_tables)
            .is_some_and(|t| {
                t.iter()
                    .any(|x| x.get("term").and_then(Item::as_str) == Some(term))
            })
    }

    /// Add a term.
    ///
    /// # Errors
    /// If the term is empty or already present.
    pub fn add(&mut self, entry: &TermEntry) -> Result<(), TermsError> {
        if entry.term.trim().is_empty() {
            return Err(err("terms", "term is empty"));
        }
        if self.contains(&entry.term) {
            return Err(err(
                "terms",
                format!("term `{}` already exists", entry.term),
            ));
        }
        let mut t = Table::new();
        t["term"] = value(entry.term.as_str());
        if let Some(k) = &entry.kind {
            t["kind"] = value(k.as_str());
        }
        if !entry.variants.is_empty() {
            let mut a = Array::new();
            for v in &entry.variants {
                a.push(v.as_str());
            }
            t["variants"] = value(a);
        }
        if entry.ignore_case {
            t["ignore_case"] = value(true);
        }
        self.tables().push(t);
        Ok(())
    }

    /// Remove a term by canonical spelling. Returns whether one was removed.
    pub fn remove(&mut self, term: &str) -> bool {
        let tables = self.tables();
        let Some(i) = tables
            .iter()
            .position(|x| x.get("term").and_then(Item::as_str) == Some(term))
        else {
            return false;
        };
        tables.remove(i);
        true
    }
}

/// Whether `add` should treat a term as case-insensitive when no flag says so:
/// an all-lowercase term has no canonical capitalisation to enforce.
#[must_use]
pub fn infer_ignore_case(term: &str) -> bool {
    !term.chars().any(char::is_uppercase)
}
