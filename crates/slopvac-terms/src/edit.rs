//! Comment-preserving edits to a terms file.
//!
//! Both TOML spellings of the `terms` list work: `[[terms]]` tables and an
//! inline `terms = [{ term = "..." }, ...]` array. Saves are atomic: the
//! document is written to a temporary file in the same directory and renamed
//! over the target, so a failed save leaves the original untouched.

use std::path::Path;

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value, value};

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

fn fields(entry: &TermEntry) -> Vec<(&'static str, Value)> {
    let mut out = vec![("term", Value::from(entry.term.as_str()))];
    if let Some(k) = &entry.kind {
        out.push(("kind", Value::from(k.as_str())));
    }
    if !entry.variants.is_empty() {
        let mut a = Array::new();
        for v in &entry.variants {
            a.push(v.as_str());
        }
        out.push(("variants", Value::from(a)));
    }
    if entry.ignore_case {
        out.push(("ignore_case", Value::from(true)));
    }
    if entry.case_sensitive {
        out.push(("case_sensitive", Value::from(true)));
    }
    out
}

fn inline_term(t: &InlineTable) -> Option<&str> {
    t.get("term").and_then(Value::as_str)
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

    /// Write the document atomically, creating parent directories.
    ///
    /// # Errors
    /// Write failures; the original file is left as it was.
    pub fn save(&self, path: &Path) -> Result<(), TermsError> {
        let io = |e: &std::io::Error| TermsError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        };
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| io(&e))?;
        let name = path.file_name().map_or_else(
            || "terms.toml".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
        let written =
            std::fs::write(&tmp, self.doc.to_string()).and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(io(&e));
        }
        Ok(())
    }

    /// Render the document.
    #[must_use]
    pub fn render(&self) -> String {
        self.doc.to_string()
    }

    /// Canonical spellings in file order.
    #[must_use]
    pub fn terms(&self) -> Vec<String> {
        match self.doc.get("terms") {
            Some(Item::ArrayOfTables(t)) => t
                .iter()
                .filter_map(|x| x.get("term").and_then(Item::as_str).map(str::to_owned))
                .collect(),
            Some(Item::Value(Value::Array(a))) => a
                .iter()
                .filter_map(|v| v.as_inline_table().and_then(inline_term).map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether a term with this canonical spelling exists.
    #[must_use]
    pub fn contains(&self, term: &str) -> bool {
        self.terms().iter().any(|t| t == term)
    }

    /// Add a term in whichever form the file already uses.
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
        match self.doc.get_mut("terms") {
            Some(Item::Value(Value::Array(a))) => {
                let mut t = InlineTable::new();
                for (k, v) in fields(entry) {
                    t.insert(k, v);
                }
                a.push(t);
            }
            Some(Item::ArrayOfTables(tables)) => tables.push(Self::table(entry)),
            _ => {
                let mut tables = ArrayOfTables::new();
                tables.push(Self::table(entry));
                self.doc["terms"] = Item::ArrayOfTables(tables);
            }
        }
        Ok(())
    }

    fn table(entry: &TermEntry) -> Table {
        let mut t = Table::new();
        for (k, v) in fields(entry) {
            t[k] = Item::Value(v);
        }
        t
    }

    /// Remove a term by canonical spelling. Returns whether one was removed.
    pub fn remove(&mut self, term: &str) -> bool {
        match self.doc.get_mut("terms") {
            Some(Item::ArrayOfTables(tables)) => {
                let at = tables
                    .iter()
                    .position(|x| x.get("term").and_then(Item::as_str) == Some(term));
                at.is_some_and(|i| {
                    tables.remove(i);
                    true
                })
            }
            Some(Item::Value(Value::Array(a))) => {
                let at = a
                    .iter()
                    .position(|v| v.as_inline_table().and_then(inline_term) == Some(term));
                at.is_some_and(|i| {
                    a.remove(i);
                    true
                })
            }
            _ => false,
        }
    }
}

/// Whether `add` should treat a term as case-insensitive when no flag says so:
/// an all-lowercase term has no canonical capitalisation to enforce.
#[must_use]
pub fn infer_ignore_case(term: &str) -> bool {
    !term.chars().any(char::is_uppercase)
}
