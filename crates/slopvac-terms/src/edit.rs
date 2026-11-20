//! Comment-preserving edits to a terms file.
//!
//! Both TOML spellings of the `terms` list work: `[[terms]]` tables and an
//! inline `terms = [{ term = "..." }, ...]` array. Saves are atomic: the
//! document is written to a temporary file in the same directory and renamed
//! over the target, so a failed save leaves the original untouched.

use std::io::Write;
use std::path::Path;

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value, value};

use crate::messages::TermsText;
use crate::shape::TERMS_SCHEMA_VERSION;
use crate::{EntryProblem, TermEntry, TermsError};

/// A terms file open for editing. Comments and layout survive an edit.
#[derive(Debug)]
pub struct TermsDoc {
    doc: DocumentMut,
}

fn err(path: &str, message: impl Into<String>) -> TermsError {
    TermsError::Entry {
        path: path.to_owned(),
        message: message.into(),
        number: None,
        problem: EntryProblem::Duplicate,
    }
}

impl Default for TermsDoc {
    fn default() -> Self {
        let mut doc = DocumentMut::new();
        doc["schema_version"] = value(i64::from(TERMS_SCHEMA_VERSION));
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
        Self::open_labeled(path, &path.display().to_string())
    }

    /// Like [`TermsDoc::open`], naming the file `shown` in errors. Pass a label that is
    /// relative to the project root, so an error never carries an absolute path.
    ///
    /// # Errors
    /// As [`TermsDoc::open`].
    pub fn open_labeled(path: &Path, shown: &str) -> Result<Self, TermsError> {
        let shown = shown.to_owned();
        match crate::read::read(path, &shown, crate::MAX_TERMS_FILE_BYTES) {
            Ok(text) => {
                crate::TermsFile::parse(&shown, &text)?;
                let doc = text.parse::<DocumentMut>().map_err(|e| TermsError::Parse {
                    path: shown,
                    message: e.to_string(),
                })?;
                Ok(Self { doc })
            }
            Err(crate::read::ReadFailure::Missing(_)) => Ok(Self::default()),
            Err(crate::read::ReadFailure::Error(e)) => Err(e),
        }
    }

    /// Write the document atomically, creating parent directories.
    ///
    /// The bytes go to an exclusively created, uniquely named temporary file
    /// in the target's directory, and `persist` renames it over the target. A
    /// symlinked target is followed, so the link survives. A failed save
    /// leaves the original untouched and removes only its own temporary file.
    ///
    /// # Errors
    /// Write failures; the original file is left as it was.
    pub fn save(&self, path: &Path) -> Result<(), TermsError> {
        self.save_labeled(path, &path.display().to_string())
    }

    /// Like [`TermsDoc::save`], naming the file `shown` in errors. Pass a label that is
    /// relative to the project root, so an error never carries an absolute path. The cause of
    /// a failure is reduced to its OS code or kind, because a library error can embed the
    /// path.
    ///
    /// # Errors
    /// As [`TermsDoc::save`].
    pub fn save_labeled(&self, path: &Path, shown: &str) -> Result<(), TermsError> {
        self.save_inner(path, shown, |f, bytes| f.write_all(bytes))
    }

    /// [`TermsDoc::save`] with the byte writer supplied by the caller, so a
    /// test can fail the write part-way.
    ///
    /// # Errors
    /// As [`TermsDoc::save`].
    #[doc(hidden)]
    pub fn save_with(
        &self,
        path: &Path,
        write: impl FnOnce(&mut dyn Write, &[u8]) -> std::io::Result<()>,
    ) -> Result<(), TermsError> {
        self.save_inner(path, &path.display().to_string(), write)
    }

    fn save_inner(
        &self,
        path: &Path,
        shown: &str,
        write: impl FnOnce(&mut dyn Write, &[u8]) -> std::io::Result<()>,
    ) -> Result<(), TermsError> {
        let io = |e: &std::io::Error| TermsError::Io {
            path: shown.to_owned(),
            message: crate::read::scrub(e),
        };
        let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let dir = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| io(&e))?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".terms-")
            .suffix(".tmp")
            .tempfile_in(dir)
            .map_err(|e| io(&e))?;
        // The writer is the caller's own, so its message cannot hold our path.
        write(&mut tmp, self.render().as_bytes()).map_err(|e| TermsError::Io {
            path: shown.to_owned(),
            message: e.to_string(),
        })?;
        tmp.as_file().sync_all().map_err(|e| io(&e))?;
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(tmp.path(), meta.permissions()).map_err(|e| io(&e))?;
        }
        tmp.persist(&target).map_err(|e| io(&e.error))?;
        Ok(())
    }

    /// Render the document.
    #[must_use]
    pub fn render(&self) -> String {
        // The schema requires a `terms` array. TOML omits an empty array of
        // tables, and a new document has none, so write `terms = []`.
        let mut doc = self.doc.clone();
        let empty = match doc.get("terms") {
            None => true,
            Some(Item::ArrayOfTables(t)) => t.is_empty(),
            Some(_) => false,
        };
        if empty {
            doc["terms"] = value(Array::new());
        }
        doc.to_string()
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
    /// If the entry breaks the terms schema (see [`TermEntry`]) or the term is
    /// already present. The document is unchanged on error.
    pub fn add(&mut self, entry: &TermEntry) -> Result<(), TermsError> {
        entry.validate("terms")?;
        if self.contains(&entry.term) {
            return Err(err(
                "terms",
                TermsText::TermExists {
                    term: entry.term.clone(),
                }
                .to_string(),
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
