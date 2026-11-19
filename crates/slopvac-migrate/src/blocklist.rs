//! Rewrite of the 2.x word blocklists that a config's `vocabulary.path` names.
//!
//! A 3.0 blocklist needs `schema_version = 3` and the `pos` spellings the schema lists.
//! 2.x had neither rule, so a migrated config would point at a file the 3.0 loader refuses.
//! This module plans one more change per referenced file and checks the result with the
//! real `slopvac-terms` loader before the plan is returned.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use slopvac_fs::lexical_normalize;
use slopvac_terms::{Blocklist, Pos, TermsError};
use toml_edit::{DocumentMut, Item, TableLike, Value};

use crate::config::{key_lines, line_of};
use crate::limits::{Budget, Kind};
use crate::plan::ensure_inside;
use crate::{Dropped, Error};

/// The longest part of a user's `pos` value that a message repeats.
const SHOWN_POS_CHARS: usize = 64;

/// A `vocabulary.path` in a 2.x config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reference {
    /// The key, such as `vocabulary.path` or `overrides[0].vocabulary.path`.
    pub(crate) key: String,
    /// The path as written, relative to the config's directory.
    pub(crate) path: String,
}

/// The blocklist paths a 2.x config names, at the project level and in each override.
pub(crate) fn references(config: &str) -> Vec<Reference> {
    let Ok(doc) = config.parse::<DocumentMut>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect(doc.as_table(), "", &mut out);
    if let Some(overrides) = doc.get("overrides") {
        let tables: Vec<(usize, &dyn TableLike)> = match overrides {
            Item::ArrayOfTables(aot) => aot
                .iter()
                .enumerate()
                .map(|(i, t)| (i, t as &dyn TableLike))
                .collect(),
            Item::Value(Value::Array(arr)) => arr
                .iter()
                .enumerate()
                .filter_map(|(i, v)| Some((i, v.as_inline_table()? as &dyn TableLike)))
                .collect(),
            _ => Vec::new(),
        };
        for (i, table) in tables {
            collect(table, &format!("overrides[{i}]."), &mut out);
        }
    }
    out
}

fn collect(table: &dyn TableLike, prefix: &str, out: &mut Vec<Reference>) {
    let path = table
        .get("vocabulary")
        .and_then(Item::as_table_like)
        .and_then(|v| v.get("path"))
        .and_then(Item::as_str);
    if let Some(path) = path {
        out.push(Reference {
            key: format!("{prefix}vocabulary.path"),
            path: path.to_owned(),
        });
    }
}

/// What planning one referenced blocklist decided.
#[derive(Debug)]
pub(crate) enum Planned {
    /// Nothing to do: already planned through another reference, or it loads as it is.
    Skip,
    /// Replace `old` with `new`.
    Rewrite {
        /// The path relative to the project root.
        path: PathBuf,
        old: String,
        new: String,
        notes: Vec<String>,
    },
    /// The blocklist cannot be migrated. Each record says why.
    Blocked(Vec<Dropped>),
}

/// The file a reference names, relative to the project root.
///
/// # Errors
/// [`Error::Unsafe`] when the path leaves the project root or crosses a symbolic link.
fn locate(
    root: &Path,
    dir: &Path,
    reference: &Reference,
    config: &str,
    line: usize,
) -> Result<PathBuf, Error> {
    let full = lexical_normalize(&dir.join(&reference.path));
    let base = lexical_normalize(root);
    let rel = if base == Path::new(".") {
        full.is_relative().then(|| full.clone())
    } else {
        full.strip_prefix(&base).ok().map(Path::to_path_buf)
    };
    match rel {
        Some(rel) if !matches!(rel.components().next(), Some(Component::ParentDir)) => {
            ensure_inside(root, &rel)?;
            Ok(rel)
        }
        _ => Err(Error::Unsafe(format!(
            "{config}:{line}: `{}`: the blocklist is outside the project root, so migrate does not rewrite it. \
             Move it into the project, or give it `schema_version = 3` by hand",
            reference.key
        ))),
    }
}

/// Plan the rewrite of the blocklist `reference` names. The reference sits at `line` of
/// the config `config`.
///
/// `seen` holds the files already planned, so a file two settings share is planned once.
///
/// # Errors
/// [`Error::Unsafe`] for a path outside the project, and the read errors of [`Budget::read`].
pub(crate) fn plan_reference(
    root: &Path,
    dir: &Path,
    reference: &Reference,
    config: &str,
    line: usize,
    seen: &mut BTreeSet<PathBuf>,
    budget: &mut Budget,
) -> Result<Planned, Error> {
    let rel = locate(root, dir, reference, config, line)?;
    if !seen.insert(rel.clone()) {
        return Ok(Planned::Skip);
    }
    let label = rel.display().to_string();
    let full = root.join(&rel);
    let blocked = |reason: &str| {
        Ok(Planned::Blocked(vec![Dropped::new(
            config,
            reference.key.as_str(),
            line,
            format!("blocklist `{label}`: {reason}"),
        )]))
    };
    match fs::symlink_metadata(&full) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return blocked("not a regular file"),
        Err(e) if e.kind() == ErrorKind::NotFound => return blocked("file not found"),
        Err(e) => return Err(Error::io(&full, e)),
    }
    let old = budget.read(&full, Kind::Terms)?;
    let format = full
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match rewrite(&label, &format, &old) {
        Ok(None) => Ok(Planned::Skip),
        Ok(Some((new, notes))) => {
            budget.check_text(&full, Kind::Terms, &new)?;
            Ok(Planned::Rewrite {
                path: rel,
                old,
                new,
                notes,
            })
        }
        Err(dropped) => Ok(Planned::Blocked(dropped)),
    }
}

/// How 3.0 reads a `pos` value that 2.x accepted.
#[derive(Debug, PartialEq, Eq)]
enum PosFix {
    /// 3.0 loads the value as it is.
    Keep,
    /// 2.x accepted the value after trimming or lower-casing it. 3.0 wants this long form.
    To(&'static str),
    /// 2.x did not accept the value either.
    Unknown,
}

fn long_form(pos: Pos) -> &'static str {
    match pos {
        Pos::Noun => "noun",
        Pos::Verb => "verb",
        Pos::Adjective => "adjective",
        Pos::Adverb => "adverb",
        Pos::Preposition => "preposition",
        Pos::Conjunction => "conjunction",
        Pos::Pronoun => "pronoun",
        Pos::Article => "article",
    }
}

/// 2.x trimmed the value, tried it as it was, then lower-cased it. 3.0 matches the listed
/// spellings exactly, so the two fallbacks become the long form.
fn fix_pos(raw: &str) -> PosFix {
    if Pos::parse(raw).is_some() {
        return PosFix::Keep;
    }
    let trimmed = raw.trim();
    match Pos::parse(trimmed).or_else(|| Pos::parse(&trimmed.to_lowercase())) {
        Some(pos) => PosFix::To(long_form(pos)),
        None => PosFix::Unknown,
    }
}

fn shown_pos(raw: &str) -> String {
    let mut out: String = raw.chars().take(SHOWN_POS_CHARS).collect();
    if raw.chars().nth(SHOWN_POS_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// The entry tables with their index in `entries`, the index `key_lines` uses.
fn entry_tables(entries: &mut Item) -> Vec<(usize, &mut dyn TableLike)> {
    match entries {
        Item::ArrayOfTables(aot) => aot
            .iter_mut()
            .enumerate()
            .map(|(i, t)| (i, t as &mut dyn TableLike))
            .collect(),
        Item::Value(Value::Array(arr)) => arr
            .iter_mut()
            .enumerate()
            .filter_map(|(i, v)| Some((i, v.as_inline_table_mut()? as &mut dyn TableLike)))
            .collect(),
        _ => Vec::new(),
    }
}

/// Rewrite one blocklist to 3.0. `None` when the 3.0 loader already takes the text.
///
/// Only TOML is rewritten: the edit keeps its comments and layout. A JSON or YAML blocklist
/// that 3.0 refuses is reported, because a text edit of those formats could change what the
/// file means. Line numbers refer to `text`, which is what the user has on disk.
fn rewrite(
    label: &str,
    format: &str,
    text: &str,
) -> Result<Option<(String, Vec<String>)>, Vec<Dropped>> {
    let blocked =
        |line: usize, key: &str, reason: String| vec![Dropped::new(label, key, line, reason)];
    let Err(refused) = Blocklist::parse(label, format, text) else {
        return Ok(None);
    };
    if format != "toml" {
        return Err(blocked(
            1,
            "schema_version",
            format!(
                "3.0 refuses this .{format} blocklist ({}), and migrate rewrites TOML blocklists only. \
                 Add `schema_version: 3` and long-form `pos` values by hand, or convert it to TOML",
                describe(&refused)
            ),
        ));
    }
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        blocked(
            e.span().map_or(1, |s| line_of(text, s.start)),
            "",
            e.message().to_owned(),
        )
    })?;
    let lines = key_lines(text).unwrap_or_default();
    let line = |key: &str| lines.get(key).copied().unwrap_or(1);
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    match doc.get("schema_version") {
        None => {
            doc.insert("schema_version", toml_edit::value(3));
            notes.push(format!("{label}: added schema_version = 3"));
        }
        Some(item) if item.as_integer() == Some(3) => {}
        Some(_) => problems.extend(blocked(
            line("schema_version"),
            "schema_version",
            "a 2.x blocklist has no `schema_version`; this value is not 3, so migrate leaves it for you to fix"
                .to_owned(),
        )),
    }
    if let Some(entries) = doc.get_mut("entries") {
        for (index, table) in entry_tables(entries) {
            let Some(raw) = table.get("pos").and_then(Item::as_str).map(str::to_owned) else {
                continue;
            };
            let key = format!("entries[{index}]");
            let pos_line = lines
                .get(&format!("{key}.pos"))
                .copied()
                .unwrap_or_else(|| line(&key));
            match fix_pos(&raw) {
                PosFix::Keep => {}
                PosFix::To(long) => {
                    if let Some(item) = table.get_mut("pos") {
                        let mut value = Value::from(long);
                        if let Some(old) = item.as_value() {
                            *value.decor_mut() = old.decor().clone();
                        }
                        *item = Item::Value(value);
                    }
                    notes.push(format!(
                        "{label}:{pos_line}: pos `{}` rewritten to `{long}`",
                        shown_pos(&raw)
                    ));
                }
                PosFix::Unknown => problems.extend(blocked(
                    pos_line,
                    &format!("{key}.pos"),
                    format!(
                        "unrecognized part of speech `{}`; use noun, verb, adjective, adverb, \
                         preposition, conjunction, pronoun or article",
                        shown_pos(&raw)
                    ),
                )),
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let new = doc.to_string();
    // The real loader has the last word, so a file migrate writes always loads.
    match Blocklist::parse(label, "toml", &new) {
        Ok(_) => Ok(Some((new, notes))),
        Err(e) => Err(blocked_by(label, &e, &lines)),
    }
}

/// A loader failure as a located record. An entry error points at the entry in the file on
/// disk; the loader numbers entries from one.
fn blocked_by(label: &str, error: &TermsError, lines: &HashMap<String, usize>) -> Vec<Dropped> {
    let (key, line) = match error {
        TermsError::Entry {
            number: Some(n), ..
        } => {
            let key = format!("entries[{}]", n.saturating_sub(1));
            let line = lines.get(&key).copied().unwrap_or(1);
            (key, line)
        }
        _ => (
            "entries".to_owned(),
            lines.get("entries").copied().unwrap_or(1),
        ),
    };
    vec![Dropped::new(
        label,
        key,
        line,
        format!("3.0 refuses the migrated blocklist: {}", describe(error)),
    )]
}

/// A loader failure without the text of the entry it names.
fn describe(error: &TermsError) -> String {
    match error {
        TermsError::Entry {
            problem,
            number: Some(n),
            ..
        } => format!("entry {n} {}", problem.describe()),
        TermsError::Entry { problem, .. } => format!("an entry {}", problem.describe()),
        TermsError::MissingSchemaVersion { .. } => "no `schema_version`".to_owned(),
        TermsError::SchemaVersion { found, .. } => format!("`schema_version` is {found}"),
        TermsError::UnknownKey { key, .. } => format!("unknown key `{key}`"),
        TermsError::Parse { message, .. } => message.clone(),
        other => other.to_string(),
    }
}
